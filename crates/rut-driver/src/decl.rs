//! `.d.rut` surface lowering: a declaration
//! file's `host fn`s become a mounted module's HOST surface, so a host
//! pkg is a real package directory — `rut.jsonc` (`entry.type`, no
//! `entry.lib`) plus the `.d.rut` itself. The compiler-limitation rule
//! holds at LOAD time: every signature must be concrete
//! over the crossing set, or the load refuses naming the offender.
//!
//! What is NOT lowered (yet): `host struct` records and `builtin` decls
//! stay parse-only surface — `opaque` covers host boxes, and `builtin`
//! is the engine's (core's d.rut is mounted from `Surface::core`, not
//! read from disk). An `async` host fn lowers like any other row (its
//! `is_async` flag rides the 4-tuple); the compiler weaves its call
//! sites into the host future and the embedder registers the row
//! family with `rut_vm::register_async!`.

use rut_ast::ast::{AnyTy, Ast, ItemKind, Linkage, MemberKind, NodeHandle, TypeKind};
use rut_core::types::{
    TypeId, TY_BOOL, TY_BYTES, TY_F32, TY_F64, TY_I16, TY_I32, TY_I64, TY_I8, TY_NIL,
    TY_OPAQUE, TY_OPT_BYTES, TY_OPT_OPAQUE, TY_OPT_STR, TY_STR, TY_U16, TY_U32, TY_U64, TY_U8,
};
use rut_parser::{parse, Mode};

use crate::session::{Module, ModuleBody};

/// A crossing-set type name → its boot `TypeId`: the
/// primitives, `str`/`bytes`, and `opaque`. Everything else a host
/// signature may spell is a load error.
fn crossing_ty(name: &str) -> Option<TypeId> {
    Some(match name {
        "nil" => TY_NIL,
        "bool" => TY_BOOL,
        "str" => TY_STR,
        "bytes" => TY_BYTES,
        "f32" => TY_F32,
        "f64" => TY_F64,
        "i8" => TY_I8,
        "i16" => TY_I16,
        "i32" => TY_I32,
        "i64" => TY_I64,
        "u8" => TY_U8,
        "u16" => TY_U16,
        "u32" => TY_U32,
        "u64" => TY_U64,
        "opaque" => TY_OPAQUE,
        _ => return None,
    })
}

/// The ANSWER-position crossing table (the legal-host-returns phase):
/// the plain crossing set plus the three answer optionals — `?str`,
/// `?bytes`, `?opaque` cross back nil-flattened (the
/// optionals law), each mapped to its fixed boot `Opt` row so the
/// registry's `Option<T>` SIG verifies against the same CONST (the RFC
/// 0025 join compares ids by identity). Params stay on the plain table:
/// the host side has no `Option` param impls yet, and admitting the
/// spelling there would strand the row at registration.
fn crossing_ret_ty(name: &str) -> Option<TypeId> {
    Some(match name {
        "?str" => TY_OPT_STR,
        "?bytes" => TY_OPT_BYTES,
        "?opaque" => TY_OPT_OPAQUE,
        _ => return crossing_ty(name),
    })
}

/// Render a host signature's type node as text for diagnostics — a
/// host signature is bare names over the crossing set; a nested shape
/// reports as such. `?T` renders as `?` + its inner's text (the answer
/// optionals are the one legal nesting).
fn ty_text(ast: &Ast, h: NodeHandle<AnyTy>) -> String {
    match ast.ty(h) {
        TypeKind::TyPath { segs } if segs.len() == 1 && segs[0].generics.is_empty() => {
            ast.name(segs[0].name).to_string()
        }
        TypeKind::TyPath { segs } => {
            segs.iter().map(|s| ast.name(s.name)).collect::<Vec<_>>().join("::")
        }
        TypeKind::TyOpt { inner } => format!("?{}", ty_text(ast, *inner)),
        _ => "<a nested type>".to_string(),
    }
}

/// Lower a `.d.rut` surface into a host-body [`Module`]: every
/// `host fn` becomes a bodyless host entry `(name, params, ret)` — the
/// embedding Rust binds the bodies at run time. Non-`host` items are
/// surface the compiler consumes elsewhere; parse failures and
/// non-crossing signatures are load errors.
pub fn lower_decl_module(src: &str, origin: &str) -> Result<Module, String> {
    let (ast, diags) = parse(src, Mode::Decl);
    if !diags.is_empty() {
        let msgs: Vec<String> = diags.iter().map(|d| d.msg.clone()).collect();
        return Err(format!("{origin}: the surface does not parse: {}", msgs.join("; ")));
    }
    let mut host_funcs: Vec<(String, Vec<TypeId>, TypeId, bool)> = Vec::new();
    for it in ast.module_items(ast.root).to_vec() {
        let ItemKind::SurfaceFn { linkage: Linkage::Host, is_async, name, params, ret, .. } = ast.item(it)
        else {
            continue;
        };
        let fname = ast.name(*name).to_string();
        let mut ptys = Vec::new();
        for &p in params {
            let MemberKind::Param(pd) = ast.param(p) else {
                return Err(format!(
                    "{origin}: host fn `{fname}`: a `self` receiver cannot cross the host boundary"
                ));
            };
            let Some(th) = pd.ty else {
                return Err(format!(
                    "{origin}: host fn `{fname}`: a host signature is concrete — every parameter is typed"
                ));
            };
            let tyname = ty_text(&ast, th);
            let Some(t) = crossing_ty(&tyname) else {
                return Err(format!(
                    "{origin}: host fn `{fname}`: `{tyname}` is not a crossing type — host signatures are concrete over primitives, `str`, `bytes`, and `opaque`"
                ));
            };
            ptys.push(t);
        }
        let rty = match ret {
            Some(r) => {
                let tyname = ty_text(&ast, *r);
                crossing_ret_ty(&tyname).ok_or_else(|| {
                    format!(
                        "{origin}: host fn `{fname}`: return `{tyname}` is not a crossing type — returns cross over primitives, `str`, `bytes`, `opaque`, and the answer optionals `?str`/`?bytes`/`?opaque`"
                    )
                })?
            }
            None => TY_NIL,
        };
        host_funcs.push((fname, ptys, rty, *is_async));
    }
    Ok(Module {
        body: ModuleBody::Host { host_funcs, consts: vec![], native_types: vec![], native_traits: vec![], native_fns: vec![], native_impls: vec![] },
        ..Default::default()
    })
}
