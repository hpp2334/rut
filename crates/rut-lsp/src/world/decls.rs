//! The per-file decl doors — [`DeclMap`] (name → node address, one
//! pass per file version) and the inventory views built on it. The
//! doors are questions on a [`File`]: the views carry the owning file
//! and the node address, deriving every other fact on read.

use rut_ast::{
    AnyHandle, BuiltinPrimitiveNode, BuiltinTyNode, ItemRef, MethodRef, SurfaceFnNode, UseNode,
};

use crate::doc::Document;
use crate::world::views::{FnView, LetView, ModView, TyView, UseView};
use crate::world::File;

/// one file's decl inventory: the node ADDRESSES in source order, the
/// name lookups resolving through the interner at query time. Built
/// once per file version (at mount / at sync); views derive every
/// other fact on read.
#[derive(Debug, Default)]
pub(crate) struct DeclMap {
    /// type decls — class/struct/interface/enum/builtin/primitive/
    /// host-struct/alias, in source order
    types: Vec<AnyHandle>,
    /// fns — free fns, surface fns, and impl-block methods (an impl
    /// method's owner derives from its parent impl's target), in
    /// source order
    fns: Vec<AnyHandle>,
    /// module-scope lets
    lets: Vec<AnyHandle>,
    /// use edges (one row per imported name rides the view, not here)
    uses: Vec<AnyHandle>,
    /// `mod NAME;` namespace edges
    mods: Vec<AnyHandle>,
}

/// The decl map builder — the small pass that replaced the row
/// extractor: walk the root items once, record the node addresses.
/// Error slots render no rows (errors are the panel's business).
pub(crate) fn decls_of(doc: &Document) -> DeclMap {
    let ast = doc.ast();
    let mut m = DeclMap::default();
    for item in ast.root().items(ast) {
        match item {
            ItemRef::Class(h) => m.types.push(h.any()),
            ItemRef::Struct(h) => m.types.push(h.any()),
            ItemRef::Interface(h) => m.types.push(h.any()),
            ItemRef::Enum(h) => m.types.push(h.any()),
            ItemRef::BuiltinTy(h) => m.types.push(h.any()),
            ItemRef::BuiltinPrimitive(h) => m.types.push(h.any()),
            ItemRef::SurfaceStruct(h) => m.types.push(h.any()),
            ItemRef::Alias(h) => m.types.push(h.any()),
            ItemRef::Fn(h) => m.fns.push(h.any()),
            ItemRef::SurfaceFn(h) => m.fns.push(h.any()),
            // the inherent impl owns its methods — they ride the fns
            // list at the impl's own position
            ItemRef::Impl(h) => {
                for meth in &h.get(ast).methods {
                    let MethodRef::Method(mh) = meth else {
                        continue; // the recovery slot renders no row
                    };
                    m.fns.push(mh.any());
                }
            }
            ItemRef::ModuleLet(h) => m.lets.push(h.any()),
            ItemRef::Use(h) => m.uses.push(h.any()),
            ItemRef::ModDecl(h) => m.mods.push(h.any()),
            ItemRef::Module(_) | ItemRef::Error(_) => {}
        }
    }
    m
}

impl File {
    // ---- per-file decl views, in source order ----

    /// the use-edge node handles, in source order — the one lane that
    /// reads the raw handles (the use-both gate's document side); every
    /// other reader rides [`File::uses`]' fan-out
    pub(crate) fn use_handles(&self) -> &[AnyHandle] {
        &self.decls.uses
    }

    pub(crate) fn types(&self) -> Vec<TyView<'_>> {
        self.decls
            .types
            .iter()
            .map(|&h| TyView { file: self, h })
            .collect()
    }

    pub(crate) fn fns(&self) -> Vec<FnView<'_>> {
        self.decls
            .fns
            .iter()
            .map(|&h| FnView { file: self, h })
            .collect()
    }

    pub(crate) fn lets(&self) -> Vec<LetView<'_>> {
        self.decls
            .lets
            .iter()
            .map(|&h| LetView { file: self, h })
            .collect()
    }

    /// use views — one per imported name (a brace list fans out, the
    /// names in order)
    pub(crate) fn uses(&self) -> Vec<UseView<'_>> {
        let mut out = Vec::new();
        for &h in &self.decls.uses {
            let Some(u) = h.downcast::<UseNode>() else {
                continue;
            };
            for i in 0..u.get(self.doc.ast()).names.len() {
                out.push(UseView { file: self, h, i });
            }
        }
        out
    }

    pub(crate) fn mods(&self) -> Vec<ModView<'_>> {
        self.decls
            .mods
            .iter()
            .map(|&h| ModView { file: self, h })
            .collect()
    }

    /// the type decl named `name` — first match in source order
    pub(crate) fn ty(&self, name: &str) -> Option<TyView<'_>> {
        self.types().into_iter().find(|t| t.name() == name)
    }

    /// the names whose decl spelled the import-gated builtin linkage
    /// (`pub builtin` — the core disposal pair): completion offers them
    /// only when the document's `use` names them, the same ambient
    /// split the compiler binds by. DERIVED at read from the modifier
    /// chains — never stored.
    pub(crate) fn pub_gated(&self) -> Vec<String> {
        let ast = self.doc.ast();
        let mut out = Vec::new();
        for &h in &self.decls.fns {
            if let Some(d) = h.downcast::<SurfaceFnNode>() {
                let d = d.get(ast);
                if matches!(d.linkage(), rut_ast::Linkage::Builtin { ambient: false }) {
                    out.push(ast.name(d.name.ident).to_string());
                }
            }
        }
        for &h in &self.decls.types {
            if let Some(d) = h.downcast::<BuiltinTyNode>() {
                if !d.get(ast).ambient() {
                    out.push(ast.name(d.get(ast).name.ident).to_string());
                }
            } else if let Some(d) = h.downcast::<BuiltinPrimitiveNode>() {
                if !d.get(ast).ambient() {
                    out.push(ast.name(d.get(ast).name.ident).to_string());
                }
            }
        }
        out
    }
}
