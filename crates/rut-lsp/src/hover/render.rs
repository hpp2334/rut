//! Markdown rendering — verbatim-signature code blocks, provenance
//! lines, doc paragraphs; a candidate list on ambiguity.

use super::types::{DefIndex, FnDef, MemberSrc, TyDef, TyForm};

fn code_block(body: &str) -> String {
    format!("```rut\n{body}\n```")
}

fn provenance(origin: &str, line: u32) -> String {
    format!("— {origin}:{line}")
}

pub(crate) fn render_ty(i: &DefIndex, ty: &TyDef) -> String {
    let mut out = String::new();
    let gens = if ty.generics.is_empty() {
        String::new()
    } else {
        format!("<{}>", ty.generics.join(", "))
    };
    match ty.form {
        TyForm::Enum => {
            let members: Vec<&str> = ty.fields.iter().map(|f| f.name.as_str()).collect();
            out.push_str(&code_block(&format!(
                "enum {}{} {{ {} }}",
                ty.name,
                gens,
                members.join(", ")
            )));
        }
        TyForm::Trait | TyForm::Builtin | TyForm::BuiltinIface => {
            let mut body = String::new();
            for m in &ty.methods {
                body.push_str("    ");
                body.push_str(&m.src);
                body.push('\n');
            }
            out.push_str(&code_block(&format!(
                "{} {}{} {{\n{}}}",
                ty.form.keyword(),
                ty.name,
                gens,
                body
            )));
        }
        TyForm::Class | TyForm::Dataclass | TyForm::HostDataclass => {
            let mut body = String::new();
            for f in &ty.fields {
                body.push_str("    ");
                body.push_str(&f.src);
                body.push('\n');
            }
            out.push_str(&code_block(&format!(
                "{} {}{} {{\n{}}}",
                ty.form.keyword(),
                ty.name,
                gens,
                body
            )));
            if !ty.methods.is_empty() {
                out.push_str(&format!(
                    "\n{} method{} — hover one for its signature",
                    ty.methods.len(),
                    if ty.methods.len() == 1 { "" } else { "s" }
                ));
            }
        }
    }
    if !i.origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(&i.origin, ty.line)));
    }
    for d in &ty.doc {
        out.push_str(&format!("\n\n{}", d));
    }
    out
}

pub(crate) fn render_member(i: &DefIndex, ty: &TyDef, m: &MemberSrc, via: Option<String>) -> String {
    let mut out = code_block(&m.src);
    match via {
        Some(v) => out.push_str(&format!("\nfrom `{v}`")),
        None if ty.form == TyForm::Trait => {
            out.push_str(&format!("\ndeclared in `{}`", ty.name));
        }
        None => out.push_str(&format!("\nin `{}`", ty.name)),
    }
    if !i.origin.is_empty() {
        out.push_str(&format!("\n{}", provenance(&i.origin, m.line)));
    }
    for d in &m.doc {
        out.push_str(&format!("\n\n{}", d));
    }
    out
}

pub(crate) fn render_fn_hits(hits: &[(&DefIndex, &FnDef)], one: &FnDef) -> String {
    let i = hits.iter().find(|(_, f)| f.span == one.span).map(|(i, _)| *i);
    let mut out = code_block(&one.src);
    if let Some(o) = &one.owner {
        out.push_str(&format!("\nin `{o}`"));
    }
    if let Some(i) = i {
        if !i.origin.is_empty() {
            out.push_str(&format!("\n{}", provenance(&i.origin, one.line)));
        }
    }
    for d in &one.doc {
        out.push_str(&format!("\n\n{}", d));
    }
    out
}

pub(crate) fn render_candidates(many: &[(&DefIndex, &FnDef)]) -> String {
    let mut out = String::from("multiple definitions:");
    for (i, f) in many {
        let where_ = match &f.owner {
            Some(o) => format!(" ({o})"),
            None => String::new(),
        };
        out.push_str(&format!(
            "\n- `{}`{} — {}:{}",
            f.src.replace('\n', " "),
            where_,
            i.origin,
            f.line
        ));
    }
    out
}
