//! `.d.rut` surface lowering (RFC 0029 §2 / RFC 0025): a declaration
//! file's `host fn`s become a mounted module's HOST surface, so a host
//! pkg is a real package directory — `rut.toml` (`entry.type`, no
//! `entry.lib`) plus the `.d.rut` itself. The compiler-limitation rule
//! holds at LOAD time (RFC 0023 §1): every signature must be concrete
//! over the crossing set, or the load refuses naming the offender.
//!
//! What is NOT lowered (yet): `host struct` records and `builtin` decls
//! stay parse-only surface — `opaque` covers host boxes, and `builtin`
//! is the engine's (core's d.rut is mounted from `Surface::core`, not
//! read from disk).

use rut_ast::ast::{AnyTy, Ast, ItemKind, Linkage, MemberKind, NodeHandle, TypeKind};
use rut_core::types::{
    TypeId, TY_BOOL, TY_BYTES, TY_F32, TY_F64, TY_I16, TY_I32, TY_I64, TY_I8, TY_NIL,
    TY_OPAQUE, TY_STR, TY_U16, TY_U32, TY_U64, TY_U8, TY_VAL,
};
use rut_parser::{parse, Mode};

use crate::session::Module;

/// A crossing-set type name → its boot `TypeId` (RFC 0023 §1): the
/// primitives, `str`/`bytes`, and `opaque` — plus the host-decl-only
/// `any` value lane (nmap-hostvals P3). Everything else a host
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
        // the `any` crossing (nmap-hostvals P3): HOST-DECL-only — a
        // `.d.rut` host-fn param/answer spelling (decl-mode lexing admits
        // it; .rut source keeps the name reserved). The trust law lives
        // at this decl site: the host must answer the caller's static V —
        // the dst register's own declared type (§0.8 h, the untagged-slot
        // discipline). The id is TY_VAL, deliberately NOT the VM's
        // u32::MAX TY_ANY sentinel (the survey's collision receipt).
        "any" => TY_VAL,
        _ => return None,
    })
}

/// Render a host signature's type node as text for diagnostics — a
/// host signature is bare names; a nested shape reports as such.
fn ty_text(ast: &Ast, h: NodeHandle<AnyTy>) -> String {
    match ast.ty(h) {
        TypeKind::TyPath { segs } if segs.len() == 1 && segs[0].generics.is_empty() => {
            ast.name(segs[0].name).to_string()
        }
        TypeKind::TyPath { segs } => {
            segs.iter().map(|s| ast.name(s.name)).collect::<Vec<_>>().join("::")
        }
        _ => "<a nested type>".to_string(),
    }
}

/// Lower a `.d.rut` surface into a decl-shaped [`Module`]: every
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
    let mut host_funcs: Vec<(String, Vec<TypeId>, TypeId)> = Vec::new();
    for it in ast.module_items(ast.root).to_vec() {
        let ItemKind::SurfaceFn { linkage: Linkage::Host, name, params, ret, .. } = ast.item(it)
        else {
            continue;
        };
        let fname = ast.name(*name).to_string();
        let mut ptys = Vec::new();
        for &p in params {
            let MemberKind::Param(pd) = ast.param(p) else {
                return Err(format!(
                    "{origin}: host fn `{fname}`: a `self` receiver cannot cross the host boundary (RFC 0023 §1)"
                ));
            };
            let Some(th) = pd.ty else {
                return Err(format!(
                    "{origin}: host fn `{fname}`: a host signature is concrete — every parameter is typed (RFC 0023 §1)"
                ));
            };
            let tyname = ty_text(&ast, th);
            let Some(t) = crossing_ty(&tyname) else {
                return Err(format!(
                    "{origin}: host fn `{fname}`: `{tyname}` is not a crossing type — host signatures are concrete over primitives, `str`, `bytes`, `opaque`, and the host-decl-only `any` (RFC 0023 §1; nmap-hostvals P3)"
                ));
            };
            ptys.push(t);
        }
        let rty = match ret {
            Some(r) => {
                let tyname = ty_text(&ast, *r);
                crossing_ty(&tyname).ok_or_else(|| {
                    format!(
                        "{origin}: host fn `{fname}`: return `{tyname}` is not a crossing type (RFC 0023 §1)"
                    )
                })?
            }
            None => TY_NIL,
        };
        host_funcs.push((fname, ptys, rty));
    }
    Ok(Module {
        decl: Some(src.to_string()),
        is_decl: true,
        host_funcs,
        ..Default::default()
    })
}
