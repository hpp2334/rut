//! [`FnView`] — a fn view: a free fn, a surface fn, or an impl-block
//! method (the owner derives from the parent impl's target).

use rut_ast::{AnyHandle, Ast, FnNode, ImplNode, MethodDeclNode, ParamRef, SurfaceFnNode, Vis};

use super::{doc_lines, param_names, sig_src};
use crate::world::types::{ty_head, ty_src};
use crate::world::File;

/// a fn view — a free fn, a surface fn, or an impl-block method (the
/// owner derives from the parent impl's target)
#[derive(Clone, Copy)]
pub(crate) struct FnView<'a> {
    pub(crate) file: &'a File,
    pub(crate) h: AnyHandle,
}

impl<'a> FnView<'a> {
    fn ast(&self) -> &'a Ast {
        self.file.doc.ast()
    }

    fn text(&self) -> &'a str {
        self.file.doc.src()
    }

    pub(crate) fn name(&self) -> &'a str {
        let ast = self.ast();
        macro_rules! nm {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return ast.name(d.get(ast).name.ident);
                }
            };
        }
        nm!(FnNode);
        nm!(SurfaceFnNode);
        nm!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    /// byte span of the name identifier — the definition target
    pub(crate) fn name_span(&self) -> Option<rut_lexer::span::Span> {
        let ast = self.ast();
        macro_rules! ns {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return Some(d.get(ast).name.span);
                }
            };
        }
        ns!(FnNode);
        ns!(SurfaceFnNode);
        ns!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    /// verbatim signature source
    pub(crate) fn src(&self) -> String {
        let ast = self.ast();
        let src = self.text();
        if let Some(d) = self.h.downcast::<FnNode>() {
            let d = d.get(ast);
            return sig_src(src, ast, d.span, Some(d.body));
        }
        if let Some(d) = self.h.downcast::<SurfaceFnNode>() {
            let d = d.get(ast);
            return sig_src(src, ast, d.span, None);
        }
        if let Some(d) = self.h.downcast::<MethodDeclNode>() {
            let d = d.get(ast);
            return sig_src(src, ast, d.span, d.body);
        }
        unreachable!("a fn view addresses a fn node")
    }

    /// the declared return as written, `Self` resolved to the owning
    /// impl's target (`None` = unwritten); inferred-type hovers read
    /// it for call initializers. An impl method's `-> Self` answers
    /// `owner()` — the receiver's own type, the builder-chain law
    /// (every std builder rung is `-> Self` style); a free fn's `Self`
    /// names nothing (owner is `None`) and renders the written word.
    /// Surface rows answer like every other decl — the v1 "inference
    /// does not ride the host boundary" ruling is REVERSED (the
    /// written ret is the crossing's contract; `src()` already
    /// rendered it verbatim in hovers, and the Future family types
    /// through it).
    pub(crate) fn ret(&self) -> Option<String> {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<MethodDeclNode>() {
            let d = d.get(ast);
            if matches!(&d.ret, Some(r) if r.is_self()) {
                return self.owner();
            }
            return d.ret.map(|r| ty_src(ast, r));
        }
        macro_rules! wr {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return d.get(ast).ret.map(|r| ty_src(ast, r));
                }
            };
        }
        wr!(FnNode);
        wr!(SurfaceFnNode);
        unreachable!("a fn view addresses a fn node")
    }

    /// spelled `async`? — the call lanes' Future-wrap rule keys on it
    /// (the engine's sugar law: a call to an `async` fn answers
    /// `Future<R>`, an `async { }` block mints `Future<T>`)
    pub(crate) fn is_async(&self) -> bool {
        let ast = self.ast();
        macro_rules! ay {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return d.get(ast).is_async();
                }
            };
        }
        ay!(FnNode);
        ay!(SurfaceFnNode);
        ay!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    /// the declared generic parameters, in order (`T`, `U` in
    /// `fn f<T, U>(..)`) — the call-site substitution's variable list
    pub(crate) fn generics(&self) -> Vec<String> {
        let ast = self.ast();
        macro_rules! gs {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return d
                        .get(ast)
                        .generics
                        .iter()
                        .map(|&g| ast.name(g).to_string())
                        .collect();
                }
            };
        }
        gs!(FnNode);
        gs!(SurfaceFnNode);
        gs!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    /// does the callee take a `self` receiver? Its `param_tys` slot is
    /// the leading `None` a call site's zip rides out (the args never
    /// include the receiver)
    pub(crate) fn takes_self(&self) -> bool {
        let ast = self.ast();
        macro_rules! sf {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return matches!(d.get(ast).params.first(), Some(ParamRef::SelfParam(_)));
                }
            };
        }
        sf!(FnNode);
        sf!(SurfaceFnNode);
        sf!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    /// the declared parameter types, positional (`self` renders `None`
    /// — it types as the receiver, never a substitutable text; an
    /// undeclared param renders `None` too). The call-site lanes zip
    /// these against the args' inferred texts
    pub(crate) fn param_tys(&self) -> Vec<Option<String>> {
        let ast = self.ast();
        macro_rules! pt {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return d
                        .get(ast)
                        .params
                        .iter()
                        .map(|p| match p {
                            ParamRef::Param(ph) => ph.get(ast).ty.map(|t| ty_src(ast, t)),
                            _ => None, // `self` / `mut self` / a damaged slot
                        })
                        .collect();
                }
            };
        }
        pt!(FnNode);
        pt!(SurfaceFnNode);
        pt!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    /// declared parameter names in order, `self` excluded
    pub(crate) fn params(&self) -> Vec<String> {
        let ast = self.ast();
        macro_rules! ps {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return param_names(ast, &d.get(ast).params);
                }
            };
        }
        ps!(FnNode);
        ps!(SurfaceFnNode);
        ps!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    pub(crate) fn doc_lines(&self) -> Vec<String> {
        let ast = self.ast();
        macro_rules! dd {
            ($t:ty) => {
                if let Some(d) = self.h.downcast::<$t>() {
                    return doc_lines(self.text(), &d.get(ast).doc);
                }
            };
        }
        dd!(FnNode);
        dd!(SurfaceFnNode);
        dd!(MethodDeclNode);
        unreachable!("a fn view addresses a fn node")
    }

    /// the declared visibility — the member-vis gate reads it (impl
    /// methods carry their member vis, module-private when unannotated
    /// — the compiler's member default)
    pub(crate) fn vis(&self) -> Vis {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<FnNode>() {
            return d.get(ast).vis();
        }
        if let Some(d) = self.h.downcast::<SurfaceFnNode>() {
            return d.get(ast).vis();
        }
        if let Some(d) = self.h.downcast::<MethodDeclNode>() {
            return d.get(ast).vis().unwrap_or(Vis::Self_);
        }
        unreachable!("a fn view addresses a fn node")
    }

    /// the ambient builtin linkage (`prelude builtin fn`) — DERIVED
    /// from the modifier chain at read
    pub(crate) fn ambient(&self) -> bool {
        let ast = self.ast();
        if let Some(d) = self.h.downcast::<SurfaceFnNode>() {
            return matches!(
                d.get(ast).linkage(),
                rut_ast::Linkage::Builtin { ambient: true }
            );
        }
        false
    }

    /// the owning inherent impl's target type (`Circle` — inherent
    /// impls own their methods, so an impl method renders like the
    /// type's own surface), `None` for a free fn. DERIVED from the
    /// tree: the parent of an impl method IS its impl block.
    pub(crate) fn owner(&self) -> Option<String> {
        let ast = self.ast();
        self.h.downcast::<MethodDeclNode>()?;
        let parent = ast.parent(self.h)?;
        let imp = parent.downcast::<ImplNode>()?;
        Some(ty_head(ast, imp.get(ast).target))
    }

    pub(crate) fn span(&self) -> rut_lexer::span::Span {
        self.ast().span_of(self.h)
    }

    pub(crate) fn line(&self) -> u32 {
        1 + self.file.doc.line_of(self.span().lo)
    }
}
