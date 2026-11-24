//! The typed answer — one resolved declaration. `resolve::resolve`
//! answers a caret with [`Def`]s; every feature locates and renders
//! through them ([`Def::location`] / [`Def::view`]), so hover and
//! definition cannot disagree about what an ident means. `Copy` and
//! `Eq`, so a def sits in sets and compares across files.

use ls_types::{Position, Range};

use rut_ast::{
    AliasNode, AnyHandle, Ast, BuiltinPrimitiveNode, BuiltinTyNode, ClassNode, EnumMemberNode,
    EnumNode, FieldDeclNode, FnNode, IdentId, InterfaceNode, MethodDeclNode, ModDeclNode,
    ModuleLetNode, StructNode, SurfaceFnNode, SurfaceStructNode,
};

use crate::semantic::Span;
use crate::world::collection::{FnView, LetView, MemberView, ModView, TyView};
use crate::world::{File, FileId, World};

/// one resolved jump target — the uri is the doc itself, a workspace
/// path/URI, or (for the embedded prelude) the true repo-relative
/// source path; the client face resolves it against the workspace
#[derive(Debug, Clone, PartialEq)]
pub struct DefLocation {
    pub uri: String,
    pub range: Range,
}

/// one resolved declaration — the walk's answer to "what does the
/// caret name"
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Def {
    /// a declaration node: type, alias, fn, method, field, enum member,
    /// module let, `mod` edge
    Node { file: FileId, node: AnyHandle },
    /// a fn-local binding: param, `let`, loop variable — a row of
    /// `doc.bindings()`
    Binding { file: FileId, row: usize },
    /// a package root or a file module
    Module { file: FileId },
}

/// the view a [`Def`] addresses, for rendering
pub(crate) enum View<'a> {
    Ty(TyView<'a>),
    Fn(FnView<'a>),
    Let(LetView<'a>),
    Member(MemberView<'a>),
    Mod(ModView<'a>),
}

/// the (ident, name span) a def's node declares — the family the
/// classification wraps (its `Decl` arm) and the answer reads
pub(crate) fn decl_ident(ast: &Ast, h: AnyHandle) -> Option<(IdentId, Span)> {
    macro_rules! d {
        ($t:ty) => {
            if let Some(n) = h.downcast::<$t>() {
                let dd = n.get(ast);
                return Some((dd.name.ident, dd.name.span));
            }
        };
    }
    d!(ClassNode);
    d!(StructNode);
    d!(InterfaceNode);
    d!(EnumNode);
    d!(BuiltinTyNode);
    d!(BuiltinPrimitiveNode);
    d!(SurfaceStructNode);
    d!(AliasNode);
    d!(FnNode);
    d!(SurfaceFnNode);
    d!(MethodDeclNode);
    d!(FieldDeclNode);
    d!(EnumMemberNode);
    d!(ModuleLetNode);
    d!(ModDeclNode);
    None
}

/// the ONE jump-target constructor — byte span → LSP range over the
/// TARGET file's own text; a location's uri is its file's (`src_path`,
/// else the origin spelled as a uri). The range joins the file's own
/// doc doors directly (resolve names no higher layer — the query
/// layer's `analysis::range` stays the hover decoration's door).
pub(crate) fn loc_at(f: &File, span: Span) -> DefLocation {
    let (l1, c1) = f.doc.position(span.lo);
    let (l2, c2) = f.doc.position(span.hi);
    DefLocation {
        uri: f
            .stamp
            .src_path
            .clone()
            .unwrap_or_else(|| origin_uri(&f.stamp.origin)),
        range: Range {
            start: Position {
                line: l1,
                character: c1,
            },
            end: Position {
                line: l2,
                character: c2,
            },
        },
    }
}

/// a target file's origin as a URI — workspace origins are already
/// paths or `file://` URIs; the prelude's provenance label never
/// becomes a jump target (it carries `src_path` instead)
fn origin_uri(origin: &str) -> String {
    if origin.contains("://") {
        origin.to_string()
    } else if origin.starts_with('/') {
        format!("file://{origin}")
    } else {
        origin.to_string()
    }
}

impl Def {
    /// the file the def declares in
    pub(crate) fn file(&self) -> FileId {
        match self {
            Def::Node { file, .. } | Def::Binding { file, .. } | Def::Module { file } => *file,
        }
    }

    /// the declared name — a node's ident, a binding row's name, a
    /// file module's display name
    #[allow(dead_code)] // the plan's answer API — the render half reads names off the cursor today; Act 4's references ride this
    pub(crate) fn name<'w>(&self, w: &'w World) -> &'w str {
        match self {
            Def::Node { file, node } => {
                let ast = w.file(*file).doc.ast();
                let (id, _) = decl_ident(ast, *node).expect("a def node declares a name");
                ast.name(id)
            }
            Def::Binding { file, row } => {
                let d = w.file(*file);
                d.doc.ast().name(d.doc.bindings()[*row].name)
            }
            // the mounted file's own name — the package's own name for
            // a root, the mod path's last segment for a child
            Def::Module { file } => {
                let f = w.file(*file);
                let path = crate::resolve::mods::mod_path_of(f);
                match f.stamp.module.as_deref() {
                    Some(p) if path.is_empty() => p,
                    _ => path.rsplit('/').next().unwrap_or(path),
                }
            }
        }
    }

    /// the span the answer decorates and definition jumps to: the
    /// declaring ident (the node's whole extent standing in for a
    /// degenerate name span — the member views' law), the binding
    /// row's decl ident, the module's file top (`Span::new(0, 0)` —
    /// the pkg-head convention)
    pub(crate) fn name_span(&self, w: &World) -> Span {
        match self {
            Def::Node { file, node } => {
                let f = w.file(*file);
                let ast = f.doc.ast();
                decl_ident(ast, *node)
                    .map(|(_, sp)| sp)
                    .filter(|sp| sp.hi > sp.lo)
                    .unwrap_or_else(|| ast.span_of(*node))
            }
            Def::Binding { file, row } => w.file(*file).doc.bindings()[*row].decl_ident_span,
            Def::Module { .. } => Span::new(0, 0),
        }
    }

    /// the def's jump target — the uri is the file's, the range is the
    /// name span
    pub(crate) fn location(&self, w: &World) -> DefLocation {
        loc_at(w.file(self.file()), self.name_span(w))
    }

    /// the view the def addresses, for rendering — `None` for what no
    /// view spells (a binding row renders through its row, a module
    /// through its file)
    pub(crate) fn view<'w>(&self, w: &'w World) -> Option<View<'w>> {
        let Def::Node { file, node } = self else {
            return None;
        };
        let f = w.file(*file);
        let ast = f.doc.ast();
        // a member of a type body: the owning type rides the parent —
        // the tree-local read the decl-site lane spells
        let member = || {
            let parent = ast.parent(*node)?;
            let ty = TyView { file: f, h: parent };
            Some(View::Member(MemberView {
                file: f,
                ty,
                h: *node,
            }))
        };
        macro_rules! ty {
            ($t:ty) => {
                if node.is::<$t>() {
                    return Some(View::Ty(TyView { file: f, h: *node }));
                }
            };
        }
        ty!(ClassNode);
        ty!(StructNode);
        ty!(InterfaceNode);
        ty!(EnumNode);
        ty!(BuiltinTyNode);
        ty!(BuiltinPrimitiveNode);
        ty!(SurfaceStructNode);
        ty!(AliasNode);
        if node.is::<FnNode>() || node.is::<SurfaceFnNode>() || node.is::<MethodDeclNode>() {
            return Some(View::Fn(FnView { file: f, h: *node }));
        }
        if node.is::<FieldDeclNode>() || node.is::<EnumMemberNode>() {
            return member();
        }
        if node.is::<ModuleLetNode>() {
            return Some(View::Let(LetView { file: f, h: *node }));
        }
        if node.is::<ModDeclNode>() {
            return Some(View::Mod(ModView { file: f, h: *node }));
        }
        None
    }
}
