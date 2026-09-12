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
}

#[derive(Clone, Debug)]
pub struct ImplDecl {
    pub trait_id: u32,
    pub target: TypeId,
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
            allow_imports: false,
            inst_map: std::collections::HashMap::new(),
            queue: Vec::new(),
        }
    }

    /// Bind an imported function before body compilation.
    pub fn add_extern_fn(&mut self, name: IdentId, func: u32, params: Vec<TypeId>, ret: TypeId) {
        self.extern_fns.insert(name, ExternFn { func, params, ret });
    }

    pub fn extern_fn(&self, name: IdentId) -> Option<&ExternFn> {
        self.extern_fns.get(&name)
    }

    pub fn err(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diag::new(span, msg));
    }

    /// The crossing rule (RFC 0023 §2): what an `entry fn` signature may
    /// carry. Primitives, `string`, `unit`, `bytes` (the binary buffer,
    /// RFC 0004), `Option`/`Result` over crossable types — and `Opaque`,
    /// the host-held box (RFC 0014): the ONE cell shape an embedder may
    /// keep and pass back. Every other cell (`TodoList`, `Vec<Todo>`,
    /// `dyn Trait`, `Vec<u8>` itself, …) stays inside the VM.
    pub fn crosses_boundary(&self, ty: TypeId) -> bool {
        match self.types.kind(ty) {
            TyKind::Unit | TyKind::Prim(_) | TyKind::Str | TyKind::Bytes | TyKind::Opaque => true,
            TyKind::Option { elem } => self.crosses_boundary(*elem),
            TyKind::Result { ok, err } => self.crosses_boundary(*ok) && self.crosses_boundary(*err),
            _ => false,
        }
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
                            "`entry fn {fname}`: parameter `{}` is `{}` — only primitives, `string`, `bytes`, `Opaque`, and `Option`/`Result` over those cross the host boundary (RFC 0023 §2)",
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
                            "`entry fn {fname}` returns `{}` — only primitives, `string`, `bytes`, `Opaque`, and `Option`/`Result` over those cross the host boundary (RFC 0023 §2)",
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
        self.find_trait(name).map(|t| t.id)
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

    pub fn mk_vec(&mut self, elem: TypeId) -> TypeId {
        let name = format!("Vec<{}>", self.types.name(elem));
        self.types.intern(RutType {
            name,
            kind: TyKind::Vec { elem },
            size: 8,
            align: 8,
        })
    }
    pub fn mk_array(&mut self, elem: TypeId) -> TypeId {
        let name = format!("Array<{}>", self.types.name(elem));
        self.types.intern(RutType {
            name,
            kind: TyKind::Array { elem },
            size: 8,
            align: 8,
        })
    }
    pub fn mk_option(&mut self, elem: TypeId) -> TypeId {
        let name = format!("Option<{}>", self.types.name(elem));
        self.types.intern(RutType {
            name,
            kind: TyKind::Option { elem },
            size: 16,
            align: 8,
        })
    }
    pub fn mk_result(&mut self, ok: TypeId, err: TypeId) -> TypeId {
        let name = format!("Result<{}, {}>", self.types.name(ok), self.types.name(err));
        self.types.intern(RutType {
            name,
            kind: TyKind::Result { ok, err },
            size: 16,
            align: 8,
        })
    }
    pub fn mk_dyn(&mut self, trait_id: u32) -> TypeId {
        let name = format!("dyn {}", self.traits[trait_id as usize].name);
        self.types.intern(RutType {
            name,
            kind: TyKind::TraitObj { trait_id },
            size: 8,
            align: 8,
        })
    }
    pub fn mk_fn_ty(&mut self, params: Vec<TypeId>, ret: TypeId) -> TypeId {
        let ps: Vec<String> = params.iter().map(|&p| self.types.name(p).to_string()).collect();
        let name = format!("fn({}) -> {}", ps.join(", "), self.types.name(ret));
        self.types.intern(RutType {
            name,
            kind: TyKind::Fn { params, ret },
            size: 8,
            align: 8,
        })
    }

    pub fn layout_of(&self, ty: TypeId) -> (u32, u32) {
        // repr-C payload size/align (RFC 0015 §4)
        match self.types.kind(ty).clone() {
            TyKind::Prim(p) => (p.width(), p.width()),
            TyKind::Data { fields } => {
                let mut off = 0u32;
                let mut align = 1u32;
                for f in &fields {
                    // composite fields are cell-handle slots, not inline
                    // payloads (RFC 0009 §1, RFC 0016 §1) — the runtime stores
                    // one slot per field, so size them as one word. This also
                    // keeps layout finite for recursive records.
                    let (fsz, fal) = if self.types.repr_of(f.ty).is_ref() {
                        (8, 8)
                    } else {
                        self.layout_of(f.ty)
                    };
                    align = align.max(fal);
                    off = (off + fal - 1) / fal * fal;
                    off += fsz.max(fal);
                }
                let size = (off + align - 1) / align * align;
                (size.max(1), align)
            }
            TyKind::Array { .. } => (8, 8),
            TyKind::Option { elem } | TyKind::Result { ok: elem, .. } => {
                let (es, ea) = self.layout_of(elem);
                ((4 + es).max(8), ea.max(4))
            }
            _ => (8, 8),
        }
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

pub fn float_suffix_ty(s: FloatSuffix) -> TypeId {
    match s {
        FloatSuffix::F32 => TY_F32,
        FloatSuffix::F64 => TY_F64,
    }
}
