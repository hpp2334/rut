//! Declarations (RFC 0030 SS2): items, exports, imports, type bodies,
//! fn/method signatures, surface stubs.

use rut_lexer::span::Span;
use rut_lexer::token::Tok;
use super::*;

impl Parser {
    pub(crate) fn parse_item(&mut self) -> Option<NodeHandle<AnyItem>> {
        let sp = self.span();
        match self.tok().clone() {
            Tok::Ident(kw) => match kw.as_str() {
                "import" => self.parse_import(),
                "export" => self.parse_export_item(),
                "let" => self.parse_module_let(Vis::Self_),
                "enum" => self.parse_enum(Vis::Self_),
                "dataclass" => self.parse_dataclass(Vis::Self_),
                "class" => self.parse_class(Vis::Self_),
                "trait" => self.parse_trait(Vis::Self_),
                "impl" => self.parse_impl(),
                "suspend" if self.at_kw2("fn") => {
                    // RFC 0030 §2: `suspend fn` — M1 parses it; the compiler
                    // rejects with a targeted M3 message
                    self.bump();
                    self.parse_fn(Vis::Self_, true)
                }
                "fn" => self.parse_fn(Vis::Self_, false),
                "host" | "extern" => self.parse_surface(sp),
                // statement keywords at module scope: RFC 0003 §1
                "if" | "while" | "for" | "return" | "when" | "break" | "continue" | "await" => {
                    self.err(sp, "statements are not allowed at module scope —modules contain declarations only (RFC 0003 §1)");
                    None
                }
                _ => {
                    self.err(sp, format!("expected a declaration, found `{kw}`"));
                    None
                }
            },
            _ => {
                let found = self.peek(0).describe();
                self.err(sp, format!("expected a declaration, found {found}"));
                None
            }
        }
    }

    pub(crate) fn parse_export_item(&mut self) -> Option<NodeHandle<AnyItem>> {
        self.bump(); // export
        let vis = if self.eat_punct(Tok::LParen) {
            let v = match self.tok().clone() {
                Tok::Ident(m) if m == "mod" => Vis::Mod,
                Tok::Ident(m) if m == "super" => Vis::Super,
                Tok::Ident(m) if m == "self" => Vis::Self_,
                _ => {
                    self.err_here("expected `mod`, `super`, or `self` in export(..)");
                    Vis::Self_
                }
            };
            self.bump();
            self.expect(Tok::RParen);
            v
        } else {
            Vis::Pub
        };
        let sp = self.span();
        match self.tok().clone() {
            Tok::Ident(kw) => match kw.as_str() {
                "let" => self.parse_module_let(vis),
                "enum" => self.parse_enum(vis),
                "dataclass" => self.parse_dataclass(vis),
                "class" => self.parse_class(vis),
                "trait" => self.parse_trait(vis),
                "suspend" if self.at_kw2("fn") => {
                    self.bump();
                    self.parse_fn(vis, true)
                }
                "fn" => self.parse_fn(vis, false),
                "host" | "extern" => self.parse_surface(sp),
                _ => {
                    self.err_here(format!("`export` must precede a declaration, found `{kw}`"));
                    None
                }
            },
            _ => {
                let found = self.peek(0).describe();
                self.err_here(format!("`export` must precede a declaration, found {found}"));
                None
            }
        }
    }

    pub(crate) fn parse_import(&mut self) -> Option<NodeHandle<AnyItem>> {
        let lo = self.bump().span.lo; // import
        self.expect(Tok::LBrace)?;
        let mut names = Vec::new();
        loop {
            if self.eat_punct(Tok::RBrace) {
                break;
            }
            let Some(n) = self.expect_ident("an imported name") else {
                self.sync_stmt();
                return None;
            };
            names.push(n);
            if !self.eat_punct(Tok::Comma) {
                self.expect(Tok::RBrace);
                break;
            }
        }
        if !self.at_kw("from") {
            self.err_here("expected `from` after the import list");
        } else {
            self.bump();
        }
        let from = match self.tok().clone() {
            Tok::Str(s) | Tok::RawStr(s) => {
                self.bump();
                s
            }
            _ => {
                self.err_here("expected a module specifier string after `from`");
                String::new()
            }
        };
        self.expect(Tok::Semi);
        Some(self.item(
            ItemKind::Import { names, from },
            Span::new(lo, self.span().hi),
        ))
    }

    pub(crate) fn parse_module_let(&mut self, vis: Vis) -> Option<NodeHandle<AnyItem>> {
        let lo = self.bump().span.lo; // let
        let name = self.expect_ident("a binding name")?;
        let ty = if self.eat_punct(Tok::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(Tok::Eq);
        let init = self.parse_expr()?;
        self.expect(Tok::Semi);
        Some(self.item(
            ItemKind::ModuleLet { vis, name, ty, init },
            Span::new(lo, self.span().hi),
        ))
    }

    pub(crate) fn parse_enum(&mut self, vis: Vis) -> Option<NodeHandle<AnyItem>> {
        let lo = self.bump().span.lo; // enum
        let name = self.expect_ident("an enum name")?;
        self.expect(Tok::LBrace);
        let mut members = Vec::new();
        loop {
            if self.eat_punct(Tok::RBrace) {
                break;
            }
            let Some(m) = self.expect_ident("an enum member") else {
                self.sync_stmt();
                return None;
            };
            let mut val = None;
            if self.eat_punct(Tok::Eq) {
                let neg = self.eat_punct(Tok::Minus);
                match self.tok().clone() {
                    Tok::Int(v, _) => {
                        self.bump();
                        val = Some(v as i64 * if neg { -1 } else { 1 });
                    }
                    _ => self.err_here("expected an integer after `=` in an enum member"),
                }
            }
            members.push((m, val));
            if !self.eat_punct(Tok::Comma) {
                self.expect(Tok::RBrace);
                break;
            }
        }
        Some(self.item(
            ItemKind::Enum { vis, name, members },
            Span::new(lo, self.span().hi),
        ))
    }

    pub(crate) fn parse_generic_params(&mut self) -> Option<Vec<IdentId>> {
        let mut out = Vec::new();
        self.expect(Tok::Lt);
        loop {
            if self.eat_punct(Tok::Gt) {
                break;
            }
            let Some(n) = self.expect_ident("a generic parameter") else {
                return Some(out);
            };
            out.push(n);
            if !self.eat_punct(Tok::Comma) {
                self.expect_gt();
                break;
            }
        }
        Some(out)
    }

    /// dataclass/class body: fields and methods. Field separators are `;` or
    /// `,` (the corpus uses both); methods need none (RFC 0009/0010).
    pub(crate) fn parse_type_body(&mut self, allow_private: bool, is_dataclass: bool) -> Option<(Vec<NodeHandle<FieldDeclNode>>, Vec<NodeHandle<MethodDeclNode>>)> {
        self.expect(Tok::LBrace)?;
        if !self.enter() {
            self.sync_stmt();
            return Some((Vec::new(), Vec::new()));
        }
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        loop {
            if self.eat_punct(Tok::RBrace) {
                break;
            }
            let before = self.pos;
            let lo = self.span();
            let mut is_private = false;
            let mut is_static = false;
            let mut is_suspend = false;
            while let Tok::Ident(m) = self.tok().clone() {
                match m.as_str() {
                    "private" => {
                        is_private = true;
                        self.bump();
                        if !allow_private {
                            self.err(lo, format!("`private` is not allowed in a dataclass —all fields are public (RFC 0009); privacy is the class's job (RFC 0010)"));
                        }
                    }
                    "static" => {
                        is_static = true;
                        self.bump();
                        if is_dataclass {
                            self.err(lo, "dataclasses have no `static` members (RFC 0009)");
                        }
                    }
                    "suspend" => {
                        is_suspend = true;
                        self.bump();
                    }
                    _ => break,
                }
            }
            match self.tok().clone() {
                Tok::Ident(kw) if kw == "fn" => {
                    if is_static {
                        self.err(lo, "there is no `static fn` —a method without `self` IS a class method (RFC 0010 §2)");
                    }
                    match self.parse_method_decl(is_private, is_suspend, true) {
                        Some(m) => methods.push(m),
                        None => self.sync_stmt(),
                    }
                }
                Tok::Ident(_) => {
                    let name = self.expect_ident("a field name").unwrap_or(IdentId(0));
                    self.expect(Tok::Colon);
                    let Some(ty) = self.parse_type() else {
                        self.sync_stmt();
                        continue;
                    };
                    let init = if self.eat_punct(Tok::Eq) {
                        Some(self.parse_expr()?)
                    } else {
                        None
                    };
                    let node = self.field_decl(
                        FieldDeclData { is_private, is_static, name, ty, init },
                        lo.to(self.span()),
                    );
                    fields.push(node);
                    if !(self.eat_punct(Tok::Semi) || self.eat_punct(Tok::Comma)) {
                        // last field before `}` —allowed; otherwise complain
                        if !matches!(self.tok(), Tok::RBrace) {
                            self.err_here("expected `;` or `,` after a field");
                            self.sync_stmt();
                        }
                    }
                }
                _ => {
                    let found = self.peek(0).describe();
                    self.err_here(format!("expected a field or method, found {found}"));
                    self.sync_stmt();
                }
            }
            if self.pos == before {
                self.bump(); // guarantee progress
            }
        }
        self.leave();
        Some((fields, methods))
    }

    pub(crate) fn parse_dataclass(&mut self, vis: Vis) -> Option<NodeHandle<AnyItem>> {
        let lo = self.bump().span.lo; // dataclass
        let name = self.expect_ident("a dataclass name")?;
        let generics = if matches!(self.tok(), Tok::Lt) {
            self.parse_generic_params()?
        } else {
            Vec::new()
        };
        let (fields, methods) = self.parse_type_body(false, true)?;
        Some(self.item(
            ItemKind::Dataclass { vis, name, generics, fields, methods },
            Span::new(lo, self.span().hi),
        ))
    }

    pub(crate) fn parse_class(&mut self, vis: Vis) -> Option<NodeHandle<AnyItem>> {
        let lo = self.bump().span.lo; // class
        let name = self.expect_ident("a class name")?;
        let generics = if matches!(self.tok(), Tok::Lt) {
            self.parse_generic_params()?
        } else {
            Vec::new()
        };
        let (fields, methods) = self.parse_type_body(true, false)?;
        Some(self.item(
            ItemKind::Class { vis, name, generics, fields, methods },
            Span::new(lo, self.span().hi),
        ))
    }

    pub(crate) fn parse_trait(&mut self, vis: Vis) -> Option<NodeHandle<AnyItem>> {
        let lo = self.bump().span.lo; // trait
        let name = self.expect_ident("a trait name")?;
        let generics = if matches!(self.tok(), Tok::Lt) {
            self.parse_generic_params()?
        } else {
            Vec::new()
        };
        let mut requires = Vec::new();
        if self.at_kw("requires") {
            self.bump();
            loop {
                let Some(t) = self.parse_type() else {
                    break;
                };
                requires.push(t);
                if !self.eat_punct(Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::LBrace)?;
        if !self.enter() {
            self.sync_stmt();
            return Some(self.item(
                ItemKind::Trait { vis, name, generics, requires, methods: Vec::new() },
                Span::new(lo, self.span().hi),
            ));
        }
        let mut methods = Vec::new();
        loop {
            if self.eat_punct(Tok::RBrace) {
                break;
            }
            if self.at_kw("fn") {
                match self.parse_method_decl(false, false, false) {
                    Some(m) => methods.push(m),
                    None => self.sync_stmt(),
                }
            } else {
                let found = self.peek(0).describe();
                self.err_here(format!("traits declare methods only —expected `fn`, found {found}"));
                self.sync_stmt();
            }
        }
        self.leave();
        Some(self.item(
            ItemKind::Trait { vis, name, generics, requires, methods },
            Span::new(lo, self.span().hi),
        ))
    }

    pub(crate) fn parse_impl(&mut self) -> Option<NodeHandle<AnyItem>> {
        let lo = self.bump().span.lo; // impl
        if self.mode == Mode::Decl {
            self.err(Span::new(lo, lo + 4), "implementation in a declaration file —`impl` blocks live in `.rut` (RFC 0029 §2)");
        }
        let trait_ref = self.parse_type()?;
        if !self.at_kw("for") {
            self.err_here("expected `for` in `impl Trait for Type`");
        } else {
            self.bump();
        }
        let target = self.parse_type()?;
        self.expect(Tok::LBrace);
        if !self.enter() {
            self.sync_stmt();
            return Some(self.item(
                ItemKind::Impl { trait_ref, target, methods: Vec::new() },
                Span::new(lo, self.span().hi),
            ));
        }
        let mut methods = Vec::new();
        loop {
            if self.eat_punct(Tok::RBrace) {
                break;
            }
            if self.at_kw("fn") {
                match self.parse_method_decl(false, false, true) {
                    Some(m) => methods.push(m),
                    None => self.sync_stmt(),
                }
            } else {
                let found = self.peek(0).describe();
                self.err_here(format!("impl blocks contain trait methods —expected `fn`, found {found}"));
                self.sync_stmt();
            }
        }
        self.leave();
        Some(self.item(
            ItemKind::Impl { trait_ref, target, methods },
            Span::new(lo, self.span().hi),
        ))
    }

    /// fn method decl. `with_body=false` for traits/surface classes (the
    /// body is `;`). RFC 0010 §2: `self` first param —instance method.
    pub(crate) fn parse_method_decl(
        &mut self,
        is_private: bool,
        is_suspend: bool,
        with_body: bool,
    ) -> Option<NodeHandle<MethodDeclNode>> {
        let lo = self.bump().span.lo; // fn
        let name = self.expect_ident("a method name")?;
        let generics = if matches!(self.tok(), Tok::Lt) {
            self.parse_generic_params()?
        } else {
            Vec::new()
        };
        let params = self.parse_params()?;
        let ret = if self.eat_punct(Tok::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        if with_body && self.mode == Mode::Impl {
            self.expect(Tok::LBrace);
            let body = self.parse_block_body(lo)?;
            Some(self.method_decl(
                MethodDeclData { is_private, is_suspend, name, generics, params, ret, body: Some(body) },
                Span::new(lo, self.span().hi),
            ))
        } else {
            self.expect(Tok::Semi);
            Some(self.method_decl(
                MethodDeclData { is_private, is_suspend, name, generics, params, ret, body: None },
                Span::new(lo, self.span().hi),
            ))
        }
    }

    pub(crate) fn parse_fn(&mut self, vis: Vis, pre_suspend: bool) -> Option<NodeHandle<AnyItem>> {
        let lo = self.bump().span.lo; // fn
        let is_suspend = pre_suspend || {
            let s = self.at_kw("suspend");
            if s {
                self.bump();
            }
            s
        };
        if self.mode == Mode::Decl {
            self.err(Span::new(lo, lo + 2), "implementation in a declaration file (RFC 0029 §2)");
        }
        let name = self.expect_ident("a function name")?;
        let generics = if matches!(self.tok(), Tok::Lt) {
            self.parse_generic_params()?
        } else {
            Vec::new()
        };
        let params = self.parse_params()?;
        let ret = if self.eat_punct(Tok::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let mut where_bounds = Vec::new();
        if self.at_kw("where") {
            self.bump();
            loop {
                let Some(t) = self.expect_ident("a generic parameter name") else {
                    break;
                };
                if !self.at_kw("requires") {
                    self.err_here("expected `requires` in a where clause (RFC 0013 §2)");
                    break;
                }
                self.bump();
                let Some(tr) = self.parse_type() else {
                    break;
                };
                where_bounds.push((t, tr));
                if !self.eat_punct(Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::LBrace);
        let body = self.parse_block_body(lo)?;
        let f = self.fn_decl(
            FnData { vis, is_suspend, name, generics, params, ret, where_bounds, body },
            Span::new(lo, self.span().hi),
        );
        Some(f.into())
    }

    /// `host fn` / `extern fn` / `host class` / `extern class` —.d.rut only
    /// (RFC 0030 §3). Mode is chosen by file extension at the entry point;
    /// surface keywords in an implementation file are rejected here.
    pub(crate) fn parse_surface(&mut self, sp: Span) -> Option<NodeHandle<AnyItem>> {
        let linkage = match self.tok().clone() {
            Tok::Ident(k) if k == "host" => Linkage::Host,
            _ => Linkage::Extern,
        };
        if self.mode == Mode::Impl {
            self.err(sp, "declaration keyword in an implementation file —`host`/`extern` belong in a `.d.rut` (RFC 0029 §2)");
        }
        let lo = self.bump().span.lo;
        match self.tok().clone() {
            Tok::Ident(k) if k == "fn" => {
                self.bump();
                let name = self.expect_ident("a function name")?;
                let generics = if matches!(self.tok(), Tok::Lt) {
                    self.parse_generic_params()?
                } else {
                    Vec::new()
                };
                let params = self.parse_params()?;
                let ret = if self.eat_punct(Tok::Colon) {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                self.expect(Tok::Semi);
                Some(self.item(
                    ItemKind::SurfaceFn { vis: Vis::Self_, linkage, name, generics, params, ret },
                    Span::new(lo, self.span().hi),
                ))
            }
            Tok::Ident(k) if k == "class" => {
                self.bump();
                let name = self.expect_ident("a class name")?;
                // extparams: `K: Hashable` (corpus) or `K requires Hashable`
                // (RFC 0030 §3) —bounds are the `requires` form's meaning only
                let mut extparams = Vec::new();
                if matches!(self.tok(), Tok::Lt) {
                    self.bump();
                    loop {
                        if self.eat_punct(Tok::Gt) {
                            break;
                        }
                        let Some(p) = self.expect_ident("a generic parameter") else {
                            break;
                        };
                        let mut bound = None;
                        if self.eat_punct(Tok::Colon) || (self.at_kw("requires") && { self.bump(); true }) {
                            bound = self.parse_type();
                        }
                        extparams.push((p, bound));
                        if !self.eat_punct(Tok::Comma) {
                            self.expect_gt();
                            break;
                        }
                    }
                }
                self.expect(Tok::LBrace);
                let mut members = Vec::new();
                loop {
                    if self.eat_punct(Tok::RBrace) {
                        break;
                    }
                    let is_suspend = if self.at_kw("suspend") {
                        self.bump();
                        true
                    } else {
                        false
                    };
                    if self.at_kw("fn") {
                        match self.parse_method_decl(false, is_suspend, false) {
                            Some(m) => members.push(m),
                            None => self.sync_stmt(),
                        }
                    } else {
                        let found = self.peek(0).describe();
                        self.err_here(format!("expected a method declaration, found {found}"));
                        self.sync_stmt();
                    }
                }
                Some(self.item(
                    ItemKind::SurfaceClass { vis: Vis::Self_, linkage, name, extparams, members },
                    Span::new(lo, self.span().hi),
                ))
            }
            _ => {
                let found = self.peek(0).describe();
                self.err_here(format!("expected `fn` or `class` after `host`/`extern`, found {found}"));
                None
            }
        }
    }

    /// Parameter list: `self`, `mut self`, `name: Type`, `mut name: Type`.
    /// Lambda-param use (types optional) shares this parser.
    pub(crate) fn parse_params(&mut self) -> Option<Vec<NodeHandle<AnyParam>>> {
        self.expect(Tok::LParen)?;
        let mut out = Vec::new();
        loop {
            if self.eat_punct(Tok::RParen) {
                break;
            }
            let lo = self.span();
            let mut is_mut = false;
            if self.at_kw("mut") && self.at_kw2("self") {
                is_mut = true;
                self.bump();
            }
            if self.at_kw("self") {
                let _ = self.bump();
                out.push(self.member(MemberKind::SelfParam(SelfParamData { is_mut }), lo));
                if !self.eat_punct(Tok::Comma) {
                    self.expect(Tok::RParen);
                    break;
                }
                continue;
            }
            if self.at_kw("mut") {
                is_mut = true;
                self.bump();
            }
            let Some(name) = self.expect_ident("a parameter name") else {
                self.sync_stmt();
                return Some(out);
            };
            let ty = if self.eat_punct(Tok::Colon) {
                Some(self.parse_type()?)
            } else {
                None
            };
            out.push(self.member(MemberKind::Param(ParamData { is_mut, name, ty }), lo.to(self.span())));
            if !self.eat_punct(Tok::Comma) {
                self.expect(Tok::RParen);
                break;
            }
        }
        Some(out)
    }

}
