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
    /// generic parameters (`interface Foo<T>`) — empty for non-generic ones
    pub generics: Vec<IdentId>,
}

#[derive(Clone, Debug)]
pub struct ImplDecl {
    pub trait_id: u32,
    /// the interface's source name (`Iter`, `Iterator`, a user interface)
    pub trait_name: IdentId,
    pub target: TypeId,
    /// `impl Trait<T> for Vec<T>`: the generic class and the target's
    /// generic parameter idents. The impl's methods are not monomorphized
    /// as standalone fns — the compiler inlines them at the use site;
    /// `None` for ordinary concrete impls.
    pub target_data: Option<(IdentId, Vec<IdentId>)>,
    /// the interface ref's type arguments, as written (`impl Iter<T>` →
    /// `[T]`, `impl Iterator<char>` → `[char]`). The element type of the
    /// sequence/iterator contracts is argument 0, resolved at the use site
    /// under the target substitution.
    pub trait_arg_nodes: Vec<NodeHandle<AnyTy>>,
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
}

/// A monomorphization instantiation: fn key + generic substitution.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Inst {
    pub key: FnKey,
    /// generic param → concrete type
    pub subst: Vec<(IdentId, TypeId)>,
}

pub struct Ctx<'a> {
    pub ast: &'a Ast,
    pub diags: Vec<Diag>,
    pub types: TypeTable,
    pub traits: Vec<TraitDesc>,
    pub trait_decls: Vec<(IdentId, TraitDeclInfo)>,
    /// the sequence-contract trait id once referenced (RFC 0012 `Iter`) —
    /// the sequence-lowering path keys on this, never on the trait's name
    pub seq_trait: Option<u32>,
    /// the iterator-contract trait id once referenced (RFC 0012 `Iterator`)
    /// — `for..of` lowers to `next` when a type implements it and not `Iter`
    pub iter_trait: Option<u32>,
    pub funcs: Vec<FuncCode>,
    pub consts: Vec<ConstVal>,
    pub exports: Vec<(String, u32)>,
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
    /// imported functions, bound before body compilation (RFC 0029 surface):
    /// name -> signature + the exporter's scope-qualified function id
    pub extern_fns: std::collections::HashMap<IdentId, ExternFn>,
    /// imported constants (native modules: `std:math`): name -> (type, bits)
    pub extern_consts: std::collections::HashMap<IdentId, (TypeId, u64)>,
    /// imported types: name -> the exporter's scope-qualified type id
    pub extern_types: std::collections::HashMap<IdentId, TypeId>,
    /// imported types that are `class` (no outside record literal)
    pub extern_classes: std::collections::HashSet<TypeId>,
    /// imported std:core builtin containers (RFC 0028): name -> constructor.
    /// The prelude is imported, never ambient — `Array`/`Option`/`Result`/
    /// `Opaque` resolve only through this map
    pub extern_native_types: std::collections::HashMap<IdentId, rut_core::binary::NativeTy>,
    /// imported std:core builtin interfaces: name -> contract
    /// (`Disposal`/`Index`/`Iterator`)
    pub extern_ifaces: std::collections::HashMap<IdentId, rut_core::binary::NativeIface>,
    /// imported std:core compiler-lowered functions (`own`, `downcast`,
    /// `assert`/`panic`, the `str`/`bytes` natives): the name is callable
    /// only when bound
    pub extern_native_fns: std::collections::HashSet<IdentId>,
    /// Bound namespace heads (`Math` for `std:math`) — the qualified
    /// access form `<namespace>.<member>` (RFC 0028). Name-generic: the
    /// LIR routes by membership here, never by a hardcoded string.
    pub extern_namespaces: std::collections::HashSet<IdentId>,
    /// instantiated generic types: id -> (decl, type args)
    pub inst_data: std::collections::HashMap<TypeId, (IdentId, Vec<TypeId>)>,
    /// monomorphization cache: (decl, type args) -> id
    pub type_inst: std::collections::HashMap<(IdentId, Vec<TypeId>), TypeId>,
    /// generic-trait instantiation cache: (trait, type args) -> trait id
    pub trait_inst: std::collections::HashMap<(IdentId, Vec<TypeId>), u32>,
    /// whether `import` is resolved by the driver (module loading on)
    pub allow_imports: bool,
    // instantiation queue
    pub inst_map: std::collections::HashMap<Inst, u32>,
    queue: Vec<Inst>,
}

/// An imported function: the exporter's scope-qualified id and signature.
#[derive(Clone, Debug)]
pub struct ExternFn {
    pub func: u32,
    pub params: Vec<TypeId>,
    pub ret: TypeId,
    /// `Some` for a compiler-lowered intrinsic (RFC 0032 §1.1 R2): no
    /// `FuncCode` — `rut-lir` expands the call inline.
    pub intrinsic: Option<rut_core::ops::Intrinsic>,
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
            diags: Vec::new(),
            types: TypeTable::boot_scoped(scope),
            traits: Vec::new(),
            trait_decls: Vec::new(),
            seq_trait: None,
            iter_trait: None,
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
            extern_ifaces: std::collections::HashMap::new(),
            extern_native_fns: std::collections::HashSet::new(),
            extern_namespaces: std::collections::HashSet::new(),
            inst_data: std::collections::HashMap::new(),
            type_inst: std::collections::HashMap::new(),
            trait_inst: std::collections::HashMap::new(),
            allow_imports: false,
            inst_map: std::collections::HashMap::new(),
            queue: Vec::new(),
        }
    }

    /// Bind an imported function before body compilation.
    pub fn add_extern_fn(&mut self, name: IdentId, func: u32, params: Vec<TypeId>, ret: TypeId) {
        self.extern_fns.insert(name, ExternFn { func, params, ret, intrinsic: None });
    }

    /// Bind an imported compiler-lowered intrinsic (no `FuncCode`).
    pub fn add_extern_intrinsic(&mut self, name: IdentId, intrinsic: rut_core::ops::Intrinsic) {
        self.extern_fns.insert(name, ExternFn { func: 0, params: vec![], ret: TY_I32, intrinsic: Some(intrinsic) });
    }

    pub fn extern_fn(&self, name: IdentId) -> Option<&ExternFn> {
        self.extern_fns.get(&name)
    }

    /// Bind an imported constant (native modules: `std:math::PI`).
    pub fn add_extern_const(&mut self, name: IdentId, ty: TypeId, bits: u64) {
        self.extern_consts.insert(name, (ty, bits));
    }

    pub fn extern_const(&self, name: IdentId) -> Option<(TypeId, u64)> {
        self.extern_consts.get(&name).copied()
    }

    /// Bind an imported type name to the exporter's scope-qualified id.
    pub fn add_extern_type(&mut self, name: IdentId, ty: TypeId, is_class: bool) {
        self.extern_types.insert(name, ty);
        if is_class {
            self.extern_classes.insert(ty);
        }
    }

    /// Bind an imported std:core builtin container (`Array`/`Option`/
    /// `Result`/`Opaque` — RFC 0028): the type constructor is the
    /// compiler's own; the binding gates the NAME.
    pub fn add_extern_native_type(&mut self, name: IdentId, kind: rut_core::binary::NativeTy) {
        self.extern_native_types.insert(name, kind);
    }

    /// Bind an imported std:core builtin interface (`Disposal`/`Index`/
    /// `Iterator`): registered as a trait on first reference, like a
    /// declared one — but only for importers that named it.
    pub fn add_extern_iface(&mut self, name: IdentId, iface: rut_core::binary::NativeIface) {
        self.extern_ifaces.insert(name, iface);
    }

    /// Bind an imported std:core compiler-lowered function (`own`,
    /// `downcast`, `assert`/`panic`, the `str`/`bytes` natives).
    pub fn add_extern_native_fn(&mut self, name: IdentId) {
        self.extern_native_fns.insert(name);
    }

    /// Bind a namespace head (RFC 0028): `import { Math } from "std:math"`.
    pub fn add_extern_namespace(&mut self, name: IdentId) {
        self.extern_namespaces.insert(name);
    }

    /// Is `name` a bound namespace head?
    pub fn is_extern_namespace(&self, name: IdentId) -> bool {
        self.extern_namespaces.contains(&name)
    }

    /// The `std:core` not-in-scope diagnostic (RFC 0028): the prelude is
    /// imported, never ambient. A v1.1-removed name diagnoses with its
    /// replacement instead. `None` when `n` is not a prelude name —
    /// the caller keeps its ordinary message.
    pub fn not_in_core_scope(&self, n: &str) -> Option<String> {
        if let Some(msg) = rut_core::binary::removed_core(n) {
            return Some(msg.to_string());
        }
        rut_core::binary::is_core_name(n).then(|| {
            format!(
                "`{n}` is not in scope — `import {{ {n} }} from \"std:core\"` (RFC 0028: the prelude is imported, never implicit)"
            )
        })
    }

    /// Import another module's type descriptors so `(scope, local)` ids
    /// resolve for typechecking and layout (RFC 0035 §1).
    pub fn import_types(
        &mut self,
        descs: Vec<RutType>,
        blocks: &[(rut_core::ScopeId, u32)],
    ) {
        self.types.import_block(descs, blocks);
    }

    pub fn err(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diag::new(span, msg));
    }

    /// The crossing rule (RFC 0023 §2): what an `entry fn` signature may
    /// carry. Primitives, `str`, `unit`, `bytes` (the binary buffer,
    /// RFC 0004), `Option`/`Result` over crossable types — and `Opaque`,
    /// the host-held box (RFC 0014): the ONE cell shape an embedder may
    /// keep and pass back. Every other cell (`TodoList`, `Vec<Todo>`,
    /// `dyn Trait`, `Vec<u8>` itself, …) stays inside the VM.
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
                            self.types.name(ty)
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
                            self.types.name(ty)
                        ),
                    );
                }
            }
        }
    }

    pub fn name(&self, id: IdentId) -> &str {
        self.ast.name(id)
    }

    /// look a name up in the interner (no interning: the AST is shared)
    pub fn lookup_name(&self, s: &str) -> Option<IdentId> {
        self.ast.interner.lookup(s)
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
        self.find_trait(name).and_then(|t| (t.id != u32::MAX).then_some(t.id))
    }
    pub fn trait_by_id(&self, id: u32) -> &TraitDesc {
        &self.traits[id as usize]
    }
    pub fn find_impl(&self, trait_id: u32, target: TypeId) -> Option<usize> {
        self.impls
            .iter()
            .position(|i| i.trait_id == trait_id && i.target == target)
    }
    pub fn impls_of(&self, target: TypeId) -> Vec<usize> {
        self.impls
            .iter()
            .enumerate()
            .filter(|(_, i)| i.target == target)
            .map(|(k, _)| k)
            .collect()
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

    pub fn mk_array(&mut self, elem: TypeId) -> TypeId {
        let name = format!("Array<{}>", self.types.name(elem));
        self.types.intern(RutType {
            name,
            kind: TyKind::Array { elem },
        })
    }
    pub fn mk_option(&mut self, elem: TypeId) -> TypeId {
        let name = format!("Option<{}>", self.types.name(elem));
        self.types.intern(RutType {
            name,
            kind: TyKind::Option { elem },
        })
    }
    pub fn mk_result(&mut self, ok: TypeId, err: TypeId) -> TypeId {
        let name = format!("Result<{}, {}>", self.types.name(ok), self.types.name(err));
        self.types.intern(RutType {
            name,
            kind: TyKind::Result { ok, err },
        })
    }
    pub fn mk_dyn(&mut self, trait_id: u32) -> TypeId {
        let name = format!("dyn {}", self.traits[trait_id as usize].name);
        self.types.intern(RutType {
            name,
            kind: TyKind::TraitObj { trait_id },
        })
    }
    /// `*T` (RFC 0005) — nil-able rc-backed pointer
    pub fn mk_ptr(&mut self, elem: TypeId) -> TypeId {
        let name = format!("*{}", self.types.name(elem));
        self.types.intern(RutType {
            name,
            kind: TyKind::Ptr { elem },
        })
    }
    /// `(A, B, ..)` (RFC 0007) — a record with numeric field names
    pub fn mk_tuple(&mut self, elems: Vec<TypeId>) -> TypeId {
        let name = format!(
            "({})",
            elems.iter().map(|&t| self.types.name(t)).collect::<Vec<_>>().join(", ")
        );
        let fields = elems
            .into_iter()
            .enumerate()
            .map(|(i, ty)| FieldInfo { name: i.to_string(), ty })
            .collect();
        self.types.intern(RutType {
            name,
            kind: TyKind::Data { fields },
        })
    }
    pub fn mk_fn_ty(&mut self, params: Vec<TypeId>, ret: TypeId) -> TypeId {
        let ps: Vec<String> = params.iter().map(|&p| self.types.name(p).to_string()).collect();
        let name = format!("fn({}) -> {}", ps.join(", "), self.types.name(ret));
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
