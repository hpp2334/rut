//! Markdown rendering — verbatim-signature code blocks, provenance
//! lines, doc paragraphs; a candidate list on ambiguity. Plus the
//! phase-1 shapes: the binding pass's identifier hover, the M7
//! primitive blurbs, and the module-let decl render. Every verbatim
//! slice is a span×src join on the OWNING file's document (the views'
//! law) — nothing rendered is stored text.

use crate::world::bindings::{BindKind, Binding};
use crate::world::collection::{
    CandRow, Candidate, FnView, LetView, MemberView, ModView, TyView, Via,
};
use crate::world::types::TyForm;
use crate::world::File;

fn code_block(body: &str) -> String {
    format!("```rut\n{body}\n```")
}

fn provenance(origin: &str, line: u32) -> String {
    format!("— {origin}:{line}")
}

pub(crate) fn render_ty(t: &TyView) -> String {
    let mut out = String::new();
    // the decl head is the view's own verbatim slice (`head()` — from
    // the first token through the name and generics, modifiers
    // included); the arms shape only the BODY layout. No synthesized
    // text: the LSP verbatim law renders source-less rows nowhere
    match t.form() {
        TyForm::Alias => {
            // `type X = A;` — the whole decl as written
            out.push_str(&code_block(&format!("{};", t.head())));
        }
        TyForm::Enum => {
            // the member rows are the source-true slices
            let members: Vec<String> = t.fields().into_iter().map(|f| f.src()).collect();
            out.push_str(&code_block(&format!(
                "{} {{ {} }}",
                t.head(),
                members.join(", ")
            )));
        }
        TyForm::Interface | TyForm::Builtin | TyForm::Primitive => {
            let mut body = String::new();
            for m in t.methods() {
                body.push_str("    ");
                body.push_str(&m.src());
                body.push('\n');
            }
            out.push_str(&code_block(&format!("{} {{\n{}}}", t.head(), body)));
        }
        // a dep's host surface renders like a class body: the consts as
        // fields, the module's fns as the method count
        TyForm::Class | TyForm::Struct | TyForm::HostStruct => {
            let mut body = String::new();
            for f in t.fields() {
                body.push_str("    ");
                body.push_str(&f.src());
                body.push('\n');
            }
            out.push_str(&code_block(&format!("{} {{\n{}}}", t.head(), body)));
            let methods = t.methods();
            if !methods.is_empty() {
                out.push_str(&format!(
                    "\n{} method{} — hover one for its signature",
                    methods.len(),
                    if methods.len() == 1 { "" } else { "s" }
                ));
            }
        }
    }
    let origin = &t.file.stamp.origin;
    if !origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(origin, t.line())));
    }
    for d in t.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}

pub(crate) fn render_member(m: &MemberView, via: Option<String>) -> String {
    let ty = &m.ty;
    let mut out = code_block(&m.src());
    match via {
        Some(v) => out.push_str(&format!("\nfrom `{v}`")),
        None if ty.form() == TyForm::Interface => {
            out.push_str(&format!("\ndeclared in `{}`", ty.name()));
        }
        None => out.push_str(&format!("\nin `{}`", ty.name())),
    }
    let origin = &m.file.stamp.origin;
    if !origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(origin, m.line())));
    }
    for d in m.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}

pub(crate) fn render_fn_hits(one: &FnView) -> String {
    let mut out = code_block(&one.src());
    if let Some(o) = one.owner() {
        out.push_str(&format!("\nin `{o}`"));
    }
    let origin = &one.file.stamp.origin;
    if !origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(origin, one.line())));
    }
    for d in one.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}

/// the designated constructor's section — appended to the class block
/// when the hover rides a class callee: the label, the verbatim impl
/// slice (`[constructor] pub fn new(..) -> Self` whole), the ctor's
/// doc lines. The door guarantees the impl is the class's OWN file's
/// (the compiler's inherent-impl rule — a cross-module impl is no
/// ctor of the class), so no provenance line rides: the class block
/// above already cites that file.
pub(crate) fn render_ctor_section(ctor: &FnView) -> String {
    let mut out = format!("constructor:\n{}", code_block(&ctor.src()));
    for d in ctor.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}

/// the newtype's synthesized mint line — a positional newtype's only
/// constructor is the compiler-provided mint, source-less BY DESIGN
/// (no ctor decl exists to slice; the LSP verbatim law renders
/// inferred/semantic facts). `None` when the decl carries no field —
/// a damaged shape renders no mint, never a wrong one.
pub(crate) fn render_newtype_mint(t: &TyView) -> Option<String> {
    t.mint_sig()
        .map(|sig| format!("constructor — the newtype mint: {sig}"))
}

/// the candidate list — one bullet per binding: the verbatim decl
/// (source-less only for a leaf that binds nothing), its kind, the
/// use edge it rode when one bound it, and the owning file's
/// provenance
pub(crate) fn render_candidates(many: &[Candidate]) -> String {
    let mut out = String::from("multiple definitions:");
    for c in many {
        out.push_str("\n- ");
        out.push_str(&c.line());
    }
    out
}

impl Candidate<'_> {
    /// one bullet: `` `pub fn mk(n: i32) -> i32` — fn — via `use m::{
    /// mk }` — file:///ws/app/src/m/mod.rut:13 ``
    fn line(&self) -> String {
        let (src, kind, provenance) = match &self.row {
            CandRow::Ty(t) => (
                t.head(),
                match t.form() {
                    TyForm::Class => "class",
                    TyForm::Struct => "struct",
                    TyForm::Interface => "interface",
                    TyForm::Enum => "enum",
                    TyForm::Builtin => "builtin",
                    TyForm::Primitive => "builtin primitive",
                    TyForm::HostStruct => "host struct",
                    TyForm::Alias => "alias",
                }
                .to_string(),
                Some((t.file.stamp.origin.clone(), t.line())),
            ),
            CandRow::Let(l) => (
                l.src(),
                "module let".to_string(),
                Some((l.file.stamp.origin.clone(), l.line())),
            ),
            CandRow::Fn(f) => (
                f.src(),
                match f.owner() {
                    Some(o) => format!("fn (method of `{o}`)"),
                    None => "fn".to_string(),
                },
                Some((f.file.stamp.origin.clone(), f.line())),
            ),
            CandRow::Mod(m) => (
                m.src(),
                "module".to_string(),
                Some((m.file.stamp.origin.clone(), m.line())),
            ),
            // a use leaf that binds nothing: no row of its own — the
            // spelling is the candidate
            CandRow::Leaf(u) => (
                u.src(),
                "use leaf (nothing binds)".to_string(),
                Some((u.file.stamp.origin.clone(), u.line())),
            ),
        };
        let mut out = format!("`{}` — {kind}", src.replace('\n', " "));
        match &self.via {
            Some(Via::Leaf(u)) => {
                out.push_str(&format!(" — via `{}`", u.src().replace('\n', " ")));
            }
            Some(Via::Glob(g)) => {
                out.push_str(&format!(" — via the glob `{}`", g.spelling()));
            }
            None => {}
        }
        if let Some((origin, line)) = provenance.filter(|(o, _)| !o.is_empty()) {
            out.push_str(&format!(" — {origin}:{line}"));
        }
        out
    }
}

/// the identifier hover — the binding row rendered: the binding as it
/// binds (`const c: ?Circle`, `let x`, `p: Point`), then the kind; an
/// INFERRED type says so (the display-side scoping ruling: declared
/// annotations render as written — the row's span join — inference is
/// labeled). `name` is the interner spelling (the rows carry ids);
/// `ty` is the resolver's declared-or-inferred answer.
pub(crate) fn binding_markdown(b: &Binding, name: &str, ty: Option<String>) -> String {
    // the binding as it binds — the head keyword rides along (`const`
    // vs `let`) for a let-form binding and a const loop var; a param
    // renders bare (params are mutable by default)
    let kw = match (b.kind, b.is_const) {
        (BindKind::Let, true) => "const ",
        (BindKind::Let, false) => "let ",
        (BindKind::IterVar, true) => "const ",
        _ => "",
    };
    let head = match &ty {
        Some(t) => format!("{kw}{name}: {t}"),
        None => format!("{kw}{name}"),
    };
    let mut out = code_block(&head);
    out.push_str(&format!("\n{}", b.kind.label()));
    if b.ty_span.is_none() && ty.is_some() {
        out.push_str(" — type inferred");
    }
    out
}

/// M7 — the numeric/bool primitives have no surface decl to index
/// (`str`/`bytes`/`opaque` do, through core's `builtin primitive`
/// statements, and those keep their rich hover): synthesize the static
/// blurb from rut-core's documented surface. Truthful constants only;
/// `None` for names this table doesn't own
pub(crate) fn render_primitive(name: &str) -> Option<String> {
    let body = match name {
        "bool" => {
            return Some(format!(
                "{}\n\nthe two booleans — `true` / `false`",
                code_block("bool")
            ));
        }
        "f32" => format!(
            "{}\n\nIEEE-754 binary32 — the unsuffixed float default\n\nliterals `1.0`, `1.0f32`",
            code_block("f32")
        ),
        "f64" => format!(
            "{}\n\nIEEE-754 binary64 — double precision\n\nliterals `1.0f64`",
            code_block("f64")
        ),
        "i8" => prim_line(name, "signed", 8, "-128 ..= 127"),
        "i16" => prim_line(name, "signed", 16, "-32768 ..= 32767"),
        "i32" => prim_line(name, "signed", 32, "-2147483648 ..= 2147483647"),
        "i64" => prim_line(
            name,
            "signed",
            64,
            "-9223372036854775808 ..= 9223372036854775807",
        ),
        "u8" => prim_line(name, "unsigned", 8, "0 ..= 255"),
        "u16" => prim_line(name, "unsigned", 16, "0 ..= 65535"),
        "u32" => prim_line(name, "unsigned", 32, "0 ..= 4294967295"),
        "u64" => prim_line(name, "unsigned", 64, "0 ..= 18446744073709551615"),
        _ => return None,
    };
    Some(body)
}

fn prim_line(name: &str, signedness: &str, bits: u32, range: &str) -> String {
    format!(
        "{}\n\n{signedness} {bits}-bit integer — {range}\n\nliterals `0`, `0{name}`; the `wrapping_*` / `saturating_*` / `checked_*` members are compiler-lowered (core's `builtin impl {name}`)",
        code_block(name)
    )
}

/// the module-let decl render — verbatim slice, provenance, doc; the
/// type line shows the annotation or the inferred head
pub(crate) fn render_let(l: &LetView) -> String {
    let mut out = code_block(&l.src());
    out.push_str("\nmodule let");
    if let Some(ty) = l.ty() {
        out.push_str(&format!(" — `{ty}`"));
    }
    let origin = &l.file.stamp.origin;
    if !origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(origin, l.line())));
    }
    for d in l.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}

/// the resolved file-module render — a use-path segment's target (or
/// any resolved module): the mounted file as the subject (the REAL
/// name for a single-file module — the origin's basename), the child
/// count, provenance
pub(crate) fn render_module_target(f: &File, path: &str) -> String {
    let file = crate::resolve::mods::display_of_single(f, path);
    let mut out = code_block(&file);
    out.push_str("\nfile module");
    match f.mods().len() {
        0 => out.push_str("\nno child modules"),
        1 => out.push_str("\n1 child module"),
        n => out.push_str(&format!("\n{n} child modules")),
    }
    if !f.stamp.origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(&f.stamp.origin, 1)));
    }
    out
}

/// the `pub? mod NAME;` decl render — the edge as spelled (the
/// verbatim slice — never re-synthesized), the kind, the mounted file
/// the namespace resolves to, and the child's own `mod` count (the
/// walkable path segments beneath it). `children` is `None` when the
/// mounted file is not in the chain (an honest dash, never a wrong
/// count).
pub(crate) fn render_mod(f: &File, m: &ModView, mounted: &str, children: Option<usize>) -> String {
    let mut out = code_block(&format!("{};", m.src()));
    out.push_str(&format!("\nfile module — `{}`", mounted));
    if let Some(n) = children {
        out.push_str(&match n {
            0 => "\nno child modules".to_string(),
            1 => "\n1 child module".to_string(),
            n => format!("\n{n} child modules"),
        });
    }
    if !f.stamp.origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(&f.stamp.origin, m.line())));
    }
    for d in m.doc_lines() {
        out.push_str(&format!("\n\n{d}"));
    }
    out
}
