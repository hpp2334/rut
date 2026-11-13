//! `.d.rut` surface lowering: a declaration
//! file's `host fn`s become a mounted module's HOST surface, so a host
//! pkg is a real package directory — `rut.jsonc` (`entry.type`, no
//! root module's source) plus the `.d.rut` itself. The compiler-limitation rule
//! holds at LOAD time: every signature must be concrete
//! over the crossing set, or the load refuses naming the offender.
//!
//! The surface may organize as a module tree: `mod net;` in the root
//! recurses into the mounted `net/mod.d.rut` (a `mod` whose child is
//! not mounted is the loud error — the parse-only silent drop is
//! gone), and every row carries the mod path it lives under
//! ([`HostRow::path`]). The registration string is
//! [`HostRow::full_name`] — `pkg::net::connect` — and an async row's
//! family (`__start`/`__yield`/`__take`/`__cancel`) expands onto that
//! same qualified name.
//!
//! What is NOT lowered (yet): `host struct` records and `builtin` decls
//! stay parse-only surface — `opaque` covers host boxes, and `builtin`
//! is the engine's (core's d.rut is mounted from `Surface::core`, not
//! read from disk). An `async` host fn lowers like any other row (its
//! `is_async` flag rides the row); the compiler weaves its call
//! sites into the host future and the embedder registers the row
//! family with `rut_vm::register_async!`.
//!
//! A surface `let` lowers as a CONST: every `ModuleLet` whose type is
//! `f64` (spelled or inferred off a float literal) and whose
//! initializer is a float literal — or its negation, `inf`/`-inf`
//! included — folds into the host body's const rows (the literal-only
//! law, D3: the surface computes nothing). Anything else refuses
//! loudly at the lowering; one const, one name — a `let` spelling a
//! name another module of the same pkg already folded is the loud
//! refusal (the rows bind BARE at the pkg's root). The manifest's
//! `consts` table is RETIRED from the directory grammar (the strict
//! lane refuses the key); the compat lane — the manifest read at the
//! zip/bundle/store boundary, where published bytes are forever old —
//! still parses its rows, and the compat-fed mounts stamp them BESIDE
//! the surface rows through [`fold_manifest_consts`] (one name spelled
//! both ways is the loud refusal — the two are alternative spellings
//! of one const, never a layering).

use std::collections::{BTreeMap, BTreeSet};

use rut_ast::{Ast, ExprRef, ItemRef, Linkage, Lit, ParamRef, TyRef, UnOp};
use rut_core::types::{
    TypeId, TY_BOOL, TY_BYTES, TY_F32, TY_F64, TY_I16, TY_I32, TY_I64, TY_I8, TY_NIL, TY_OPAQUE,
    TY_OPT_BYTES, TY_OPT_OPAQUE, TY_OPT_STR, TY_STR, TY_U16, TY_U32, TY_U64, TY_U8,
};
use rut_lexer::token::FloatSuffix;
use rut_parser::{parse_with_recover, Mode, ParseOpts};

use crate::mods::ModSource;
use crate::session::{HostRow, Mounted, MountedBody};

/// The host surface tree's module file — the Decl flavor's spelling
/// (`NAME/mod.d.rut`), the twin of a lib body tree's `mod.rut`.
pub const DECL_CHILD_FILE: &str = "mod.d.rut";

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
fn ty_text(ast: &Ast, t: TyRef) -> String {
    match t {
        TyRef::Path(h) => {
            let segs = &h.get(ast).segs;
            if segs.len() == 1 && segs[0].generics.is_empty() {
                ast.name(segs[0].name).to_string()
            } else {
                segs.iter()
                    .map(|s| ast.name(s.name))
                    .collect::<Vec<_>>()
                    .join("::")
            }
        }
        TyRef::Opt(h) => format!("?{}", ty_text(ast, h.get(ast).inner)),
        _ => "<a nested type>".to_string(),
    }
}

/// Lower a `.d.rut` surface into a host-body [`Mounted`]: every
/// `host fn` becomes a bodyless host entry carrying its mod path —
/// the embedding Rust binds the bodies at run time. `children` is the
/// MOUNTED tree (mod path → source, the `crate::mods::mount_mod_children`
/// Decl flavor's output): a `mod net;` in the surface recurses into
/// `net/mod.d.rut`'s rows, `net/tls` into `net/tls/mod.d.rut`, and so
/// on — a declared child missing from the map is the loud error (the
/// silent drop of a `mod` declaration in a `.d.rut` is fixed here).
/// Non-`host` items are surface the compiler consumes elsewhere;
/// parse failures and non-crossing signatures are load errors. The
/// mounted tree rides the pkg (`Mounted::mods`) and the root text rides
/// the body (`root_surface`) — the bundle lane and the phase-2 dep
/// tables read both.
pub fn lower_decl_module(
    src: &str,
    children: &BTreeMap<String, ModSource>,
    origin: &str,
) -> Result<Mounted, String> {
    let p = parse_with_recover(src, Mode::Decl, ParseOpts::default());
    if !p.ast().is_clean() {
        let msgs: Vec<String> = p.ast().errors().iter().map(|d| d.msg.clone()).collect();
        return Err(format!(
            "{origin}: the surface does not parse: {}",
            msgs.join("; ")
        ));
    }
    let (ast, _) = p.into_parts();
    let mut host_funcs: Vec<HostRow> = Vec::new();
    let mut consts: Vec<(String, TypeId, u64)> = Vec::new();
    // first mount wins, mirrored in the lowering: a repeated
    // `mod NAME;` names the same mounted child — lower it once
    let mut lowered: BTreeSet<String> = BTreeSet::new();
    lower_module(
        &ast,
        "",
        children,
        origin,
        &mut host_funcs,
        &mut consts,
        &mut lowered,
    )?;
    Ok(Mounted {
        body: MountedBody::Host {
            host_funcs,
            consts,
            native_types: vec![],
            native_fns: vec![],
            native_impls: vec![],
            root_surface: Some(src.to_string()),
        },
        mods: children.clone(),
        ..Default::default()
    })
}

/// One const row as the lowering folds it: the f64 bits of a float
/// literal (or its negation) — `None` when the shape is not one. This
/// is the literal-only law's READ side: `f32`-suffixed literals are
/// not the family (f64 end-to-end), and any other expression computes
/// something — the surface computes nothing.
fn const_lit_bits(ast: &Ast, e: ExprRef) -> Option<u64> {
    match e {
        ExprRef::Lit(h) => match &h.get(ast).lit {
            Lit::Float(bits, None) | Lit::Float(bits, Some(FloatSuffix::F64)) => Some(*bits),
            _ => None,
        },
        ExprRef::Unary(h) => {
            let n = h.get(ast);
            if n.op == UnOp::Neg {
                // the negation is exact at the bit level (the sign
                // flip), `-inf` and `-0.0` included
                const_lit_bits(ast, n.expr).map(|b| (-f64::from_bits(b)).to_bits())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The surface-const law's refusal text — one sentence, every shape
/// (a non-`f64` type, a non-literal initializer, an `f32` literal).
/// `lead` names the offender (`let X: i64` / `let X`).
fn const_law(display: &str, lead: &str) -> String {
    format!(
        "{display}: `{lead}` — a surface const is a FLOAT LITERAL \
         (f64, `inf`/`-inf` allowed); the surface computes nothing"
    )
}

/// Fold one module's `let` items into const rows: the type must be
/// `f64` (spelled or inferred off the literal — the family is f64
/// end-to-end), the initializer a float literal or its negation.
/// Names ride BARE at the pkg's root, so one name folds ONCE across
/// the whole pkg — a second `let` of an already-folded name (here or
/// in a sibling module) is the loud refusal, mod path aside.
fn lower_const_rows(
    ast: &Ast,
    items: &[ItemRef],
    display: &str,
    consts: &mut Vec<(String, TypeId, u64)>,
) -> Result<(), String> {
    for &it in items {
        let ItemRef::ModuleLet(h) = it else {
            continue;
        };
        let ml = h.get(ast);
        let name = ast.name(ml.name).to_string();
        if let Some(t) = ml.ty {
            let tyname = ty_text(ast, t);
            if tyname != "f64" {
                return Err(const_law(display, &format!("let {name}: {tyname}")));
            }
        }
        let Some(bits) = const_lit_bits(ast, ml.init) else {
            // inferred-off-the-literal covers the untyped spelling;
            // everything else — a call, an arithmetic shape, an `f32`
            // literal, an int/str/bool — refuses here
            return Err(const_law(display, &format!("let {name}")));
        };
        if consts.iter().any(|(n, _, _)| *n == name) {
            return Err(format!(
                "{display}: `let {name}` — the const `{name}` is already declared in this \
                 pkg (one const, one name; the rows bind bare at the pkg's root)"
            ));
        }
        consts.push((name, TY_F64, bits));
    }
    Ok(())
}

/// One module's `host fn` rows, off the module's OWN items — the
/// row-lowering both lanes share (the host tree recursion, the inline
/// form, and the lib-surface law's root probe).
fn lower_rows(ast: &Ast, path: &str, display: &str, rows: &mut Vec<HostRow>) -> Result<(), String> {
    lower_rows_in(ast, ast.root().items(ast), path, display, rows)
}

fn lower_rows_in(
    ast: &Ast,
    items: &[ItemRef],
    path: &str,
    display: &str,
    rows: &mut Vec<HostRow>,
) -> Result<(), String> {
    for &it in items {
        let ItemRef::SurfaceFn(h) = it else {
            continue;
        };
        let sf = h.get(ast);
        if !matches!(sf.linkage, Linkage::Host) {
            continue;
        }
        let fname = ast.name(sf.name).to_string();
        let mut ptys = Vec::new();
        for &p in &sf.params {
            let ParamRef::Param(pd) = p else {
                return Err(format!(
                    "{display}: host fn `{fname}`: a `self` receiver cannot cross the host boundary"
                ));
            };
            let Some(th) = pd.get(ast).ty else {
                return Err(format!(
                    "{display}: host fn `{fname}`: a host signature is concrete — every parameter is typed"
                ));
            };
            let tyname = ty_text(ast, th);
            let Some(t) = crossing_ty(&tyname) else {
                return Err(format!(
                    "{display}: host fn `{fname}`: `{tyname}` is not a crossing type — host signatures are concrete over primitives, `str`, `bytes`, and `opaque`"
                ));
            };
            ptys.push(t);
        }
        let rty = match sf.ret {
            Some(r) => {
                let tyname = ty_text(ast, r);
                crossing_ret_ty(&tyname).ok_or_else(|| {
                    format!(
                        "{display}: host fn `{fname}`: return `{tyname}` is not a crossing type — returns cross over primitives, `str`, `bytes`, `opaque`, and the answer optionals `?str`/`?bytes`/`?opaque`"
                    )
                })?
            }
            None => TY_NIL,
        };
        rows.push(HostRow {
            path: path.to_string(),
            name: fname,
            params: ptys,
            ret: rty,
            is_async: sf.is_async,
        });
    }
    Ok(())
}

/// Lower one module of the surface tree: its own `host fn`s as
/// path-carrying rows and its `let` items as const rows, then each
/// `mod NAME;` recursed (first mount wins — a repeated declaration
/// lowers its child once, `lowered` tracking the paths already
/// walked). `display` is the declaring file's diagnostics name (the
/// root's `origin`, a child's `<mod path>/mod.d.rut`); a child that
/// is not in the mounted map — or one that does not parse — is the
/// loud error.
///
/// The INLINE form (`mod NAME { .. }`, wire v12) lowers in place: the
/// nested declarations are walked at the child's mod path, no mounted
/// file involved — the consolidated declaration entry a bundle group
/// rides speaks this shape, and a normalized inline entry must lower
/// to the same rows its file-tree form did (the generation invariant
/// the pack round-trip gate pins).
fn lower_module(
    ast: &Ast,
    path: &str,
    children: &BTreeMap<String, ModSource>,
    origin: &str,
    rows: &mut Vec<HostRow>,
    consts: &mut Vec<(String, TypeId, u64)>,
    lowered: &mut BTreeSet<String>,
) -> Result<(), String> {
    let display = if path.is_empty() {
        origin.to_string()
    } else {
        format!("{path}/{DECL_CHILD_FILE}")
    };
    lower_rows(ast, path, &display, rows)?;
    lower_const_rows(ast, ast.root().items(ast), &display, consts)?;
    for it in ast.root().items(ast).to_vec() {
        match it {
            ItemRef::ModDecl(h) => {
                let d = h.get(ast);
                let name = ast.name(d.name).to_string();
                let child_path = join_path(path, &name);
                let Some(child) = children.get(&child_path) else {
                    return Err(format!(
                        "`mod {name};` in {display} — no `{child_path}` module is mounted \
                         (the loader mounts `{child_path}/{DECL_CHILD_FILE}` beside the declaring file)"
                    ));
                };
                if !lowered.insert(child_path.clone()) {
                    continue; // first mount wins — one lowering per child
                }
                let cp = parse_with_recover(&child.text, Mode::Decl, ParseOpts::default());
                if !cp.ast().is_clean() {
                    let msgs: Vec<String> =
                        cp.ast().errors().iter().map(|d| d.msg.clone()).collect();
                    return Err(format!(
                        "{child_path}/{DECL_CHILD_FILE}: the surface does not parse: {}",
                        msgs.join("; ")
                    ));
                }
                lower_module(
                    cp.ast(),
                    &child_path,
                    children,
                    origin,
                    rows,
                    consts,
                    lowered,
                )?;
            }
            ItemRef::ModInline(h) => {
                let d = h.get(ast);
                // the inline declaration module: the nested declarations
                // lower at the child's mod path — no mounted file, no
                // children-map entry. First-wins applies to the same
                // FILE-FORM edge (an inline block and a `mod NAME;` for
                // one path would double-lower); the inline walk lowers
                // its own items directly, once.
                let name = ast.name(d.name).to_string();
                let child_path = join_path(path, &name);
                if !lowered.insert(format!("inline:{child_path}")) {
                    return Err(format!(
                        "`mod {name} {{ .. }}` in {display} — the module is already declared \
                         (one declaration per name, the one-name-one-decl law)"
                    ));
                }
                lower_inline(ast, &d.items, &child_path, &display, rows, consts, lowered)?;
            }
            _ => continue,
        }
    }
    Ok(())
}

/// Lower an INLINE module's items: its `host fn` rows and `let`
/// const rows at its mod path, then its nested `mod` edges (file-form
/// refuses — no children map backs a nested file lookup; inline
/// blocks recurse). The structural twin of [`lower_module`], one
/// parse deeper.
fn lower_inline(
    ast: &Ast,
    items: &[ItemRef],
    path: &str,
    display: &str,
    rows: &mut Vec<HostRow>,
    consts: &mut Vec<(String, TypeId, u64)>,
    lowered: &mut BTreeSet<String>,
) -> Result<(), String> {
    lower_rows_in(ast, items, path, display, rows)?;
    lower_const_rows(ast, items, display, consts)?;
    for &it in items {
        let ItemRef::ModInline(h) = it else {
            continue;
        };
        let d = h.get(ast);
        let name = ast.name(d.name).to_string();
        let child_path = join_path(path, &name);
        if !lowered.insert(format!("inline:{child_path}")) {
            return Err(format!(
                "`mod {name} {{ .. }}` in {display} — the module is already declared \
                 (one declaration per name, the one-name-one-decl law)"
            ));
        }
        lower_inline(ast, &d.items, &child_path, display, rows, consts, lowered)?;
    }
    Ok(())
}

/// `a + sep + b` (or just `b` under the root)
fn join_path(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}/{name}")
    }
}

/// The lib-surface law: `host fn` text lives only in `type = "host"`
/// pkgs. Non-parsing surfaces stay inert (doc-only); a surface that
/// PARSES and declares host fns is the loud error. The walk runs it on
/// every lib pkg's declared surface, both lanes. The probe is the
/// ROOT surface, and the root is the whole law: a lib decl surface
/// mounts no children (the loader mounts no tree under a
/// surface-only lib pkg, and the compile refuses a decl-mode pkg
/// carrying mods), while an implementation body refuses `host` text
/// at parse — so `host fn` rows under a mounted tree can only ever
/// come from the host lane's own lowering.
pub fn refuse_host_rows(src: &str, origin: &str) -> Result<(), String> {
    let p = parse_with_recover(src, Mode::Decl, ParseOpts::default());
    if !p.ast().is_clean() {
        return Ok(()); // non-parsing surfaces stay inert (doc-only)
    }
    let mut rows: Vec<HostRow> = Vec::new();
    if lower_rows(p.ast(), "", origin, &mut rows).is_err() {
        return Ok(()); // a non-crossing signature is the host lane's error, not this law's
    }
    if let Some(row) = rows.first() {
        return Err(format!(
            "{origin}: `host fn {}` — a lib pkg cannot declare \
             host fns; split the rows into a `type = \"host\"` pkg \
             and depend on it",
            row.name
        ));
    }
    Ok(())
}

/// Stamp a host pkg's manifest `consts` rows onto its lowered body —
/// the merge the COMPAT-FED mounts (the bundle loader's host groups,
/// the unpacked source-zip trees, the in-memory srczip lane) fold
/// through. Only a compat parse ever fills the map (the directory
/// grammar refuses the key — the surface `pub let` is the one FORWARD
/// spelling), so a directory mount hands this an empty table and the
/// fold is its own no-op. The surface `let` rows are the primary form
/// and the retained rows ride BESIDE them; one name spelled BOTH ways
/// is the loud refusal — the two are alternative spellings of one
/// const, never a layering. f64 rows only, exactly as the compat walk
/// shaped them.
pub fn fold_manifest_consts(
    body: &mut MountedBody,
    consts: &BTreeMap<String, f64>,
    origin: &str,
) -> Result<(), String> {
    let MountedBody::Host { consts: rows, .. } = body else {
        return Ok(()); // a non-host body carries no const rows
    };
    for (name, v) in consts {
        if rows.iter().any(|(n, _, _)| n == name) {
            return Err(format!(
                "{origin}: the const `{name}` is spelled twice — on the surface \
                 (`pub let {name}: f64 = ..`) and in the manifest's `consts`; \
                 one const, one spelling"
            ));
        }
        rows.push((name.clone(), TY_F64, v.to_bits()));
    }
    Ok(())
}
