//! Sequence lowering (RFC 0012 v1.1): `s.len()`, `s[i]`, `s[i] = v`,
//! and `for (x of s)` lower to the fused element ops — there is no
//! `Index` trait and no accessor inlining.
//!
//! Recognized sequences:
//! - `Array<T>` — fused `arrget`/`arrlen`/`arrset` (RFC 0032 §1.1 R2).
//! - `str` / `bytes` — the primitive cells (`strcharat`/`strlen`,
//!   `bytesget`/`byteslen`).
//! - `Vec<T>` — builtin by shape: a record with a `buf: Array<T>` field
//!   and a `len: i32` field (RFC 0028; the std:collection class). `len`
//!   reads the live-length field; element ops go through the `buf`
//!   cell, so they alias the vector.
//!
//! `str`/`bytes` are immutable: element assignment traps at compile
//! time on them.

use super::*;

#[derive(Clone)]
pub(crate) enum SliceSource {
    /// builtin `Array<T>` — fused element ops
    Array,
    /// builtin `str` — one-codepoint `str` elements
    Str,
    /// builtin `bytes` — `u8` elements
    Bytes,
    /// a record with the Vec shape — `buf: Array<T>` + `len: i32`
    /// (field indices into the record's field list)
    DataBuf { buf_field: u32, len_field: u32 },
}

#[derive(Clone)]
pub(crate) struct SliceInfo {
    pub source: SliceSource,
    pub elem: TypeId,
}

impl SliceInfo {
    /// The length is loop-invariant: `str`/`bytes` are immutable and
    /// `Array` is fixed-size, so a `for..of` need only read it once. A
    /// `Vec`'s live length changes with `push`, so it stays live.
    pub(crate) fn fixed_len(&self) -> bool {
        !matches!(self.source, SliceSource::DataBuf { .. })
    }
}

impl<'a, 'b> FnCompiler<'a, 'b> {
    /// Resolve the `Iter` impl for `ty`, if any.
    pub(crate) fn slice_info(&mut self, ty: TypeId) -> Option<SliceInfo> {
        match self.ctx.types.kind(ty).clone() {
            TyKind::Array { elem } => Some(SliceInfo { source: SliceSource::Array, elem }),
            TyKind::Str => Some(SliceInfo { source: SliceSource::Str, elem: TY_STR }),
            TyKind::Bytes => Some(SliceInfo { source: SliceSource::Bytes, elem: TY_U8 }),
            TyKind::Data { fields } => {
                // the Vec shape (RFC 0012 v1.1): a `buf` field holding the
                // `Array` cell and a `len` field holding the live length
                let mut buf_field: Option<(u32, TypeId)> = None;
                let mut len_field: Option<u32> = None;
                for (i, f) in fields.iter().enumerate() {
                    match f.name {
                        sym::BUF if matches!(self.ctx.types.kind(f.ty), TyKind::Array { .. }) => {
                            buf_field = Some((i as u32, f.ty))
                        }
                        sym::LEN if f.ty == TY_I32 => len_field = Some(i as u32),
                        _ => {}
                    }
                }
                let (Some((buf_field, buf_ty)), Some(len_field)) = (buf_field, len_field) else {
                    return None;
                };
                let TyKind::Array { elem } = self.ctx.types.kind(buf_ty) else {
                    unreachable!("buf_field checked above")
                };
                Some(SliceInfo {
                    source: SliceSource::DataBuf { buf_field, len_field },
                    elem: *elem,
                })
            }
            _ => None,
        }
    }

    /// Resolve the `Iterator` impl for `ty`, if any: `(impl index, Item,
    /// target subst)`. The subst resolves the `next` body and its return
    /// `s.len()` — the element count.
    pub(crate) fn emit_slice_len(&mut self, recv: u16, info: &SliceInfo, sp: u32) -> TcResult<u16> {
        match &info.source {
            SliceSource::Array => {
                let dst = self.new_reg(TY_I32);
                { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::CallNat { nat: Nat::ArrLen, recv: recv, argv_off, argc, dst: dst }, sp); }
                Ok(dst)
            }
            SliceSource::Str => {
                let dst = self.new_reg(TY_I32);
                { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::CallNat { nat: Nat::StrLen, recv: recv, argv_off, argc, dst: dst }, sp); }
                Ok(dst)
            }
            SliceSource::Bytes => {
                let dst = self.new_reg(TY_I32);
                { let (argv_off, argc) = self.pool_args(&(vec![])); self.emit(Op::CallNat { nat: Nat::ArrLen, recv: recv, argv_off, argc, dst: dst }, sp); }
                Ok(dst)
            }
            SliceSource::DataBuf { len_field, .. } => {
                let dst = self.new_reg(TY_I32);
                self.emit(Op::GetF { dst, obj: recv, field: *len_field, repr: Repr::Prim(PrimTy::I32) }, sp);
                Ok(dst)
            }
        }
    }

    /// `s[i]` — element read; the VM bounds-traps.
    pub(crate) fn emit_slice_get(&mut self, recv: u16, idx: u16, info: &SliceInfo, sp: u32) -> TcResult<u16> {
        match &info.source {
            SliceSource::Array => {
                let repr = self.ctx.types.repr_of(info.elem);
                let dst = self.new_reg(info.elem);
                self.emit(Op::ArrGet { dst, arr: recv, idx, repr }, sp);
                Ok(dst)
            }
            SliceSource::Str => {
                // StrCharAt yields a char slot; convert to a 1-codepoint str
                let creg = self.new_reg(TY_CHAR);
                self.emit(Op::StrCharAt { dst: creg, s: recv, idx }, sp);
                let dst = self.new_reg(TY_STR);
                { let (argv_off, argc) = self.pool_args(&(vec![creg])); self.emit(Op::CallNat { nat: Nat::Str, recv: NOREG, argv_off, argc, dst: dst }, sp); }
                Ok(dst)
            }
            SliceSource::Bytes => {
                let repr = self.ctx.types.repr_of(TY_U8);
                let dst = self.new_reg(TY_U8);
                self.emit(Op::ArrGet { dst, arr: recv, idx, repr }, sp);
                Ok(dst)
            }
            SliceSource::DataBuf { buf_field, .. } => {
                let buf_ty = self.ctx.mk_array(info.elem);
                let buf = self.new_reg(buf_ty);
                self.emit(Op::GetF { dst: buf, obj: recv, field: *buf_field, repr: Repr::Ref }, sp);
                let repr = self.ctx.types.repr_of(info.elem);
                let dst = self.new_reg(info.elem);
                self.emit(Op::ArrGet { dst, arr: buf, idx, repr }, sp);
                Ok(dst)
            }
        }
    }

    /// `for (let v of xs)` element reference (RFC 0012 §6): a fresh
    /// one-slot box whose field 0 is the element — ref-typed elements
    /// alias the stored slot (writes through `v.f` hit the sequence);
    /// scalars box a per-iteration copy. `str`/`bytes` keep value yields.
    pub(crate) fn emit_slice_get_ref(&mut self, recv: u16, idx: u16, info: &SliceInfo, sp: u32) -> TcResult<u16> {
        let ptr_ty = self.ctx.mk_ptr(info.elem);
        let dst = self.new_reg(ptr_ty);
        match &info.source {
            SliceSource::Array => {
                self.emit(Op::ArrGetRef { dst, arr: recv, idx, ty: ptr_ty }, sp);
            }
            SliceSource::DataBuf { buf_field, .. } => {
                let buf_ty = self.ctx.mk_array(info.elem);
                let buf = self.new_reg(buf_ty);
                self.emit(Op::GetF { dst: buf, obj: recv, field: *buf_field, repr: Repr::Ref }, sp);
                self.emit(Op::ArrGetRef { dst, arr: buf, idx, ty: ptr_ty }, sp);
            }
            _ => unreachable!("ref yield on an immutable sequence"),
        }
        Ok(dst)
    }

    /// `s[i] = val` — element write. Only the concrete `Array`/`Vec` path
    /// has a mutable element; `str`/`bytes` and the read-only `Iter`
    /// contract do not.
    pub(crate) fn emit_slice_set(&mut self, recv: u16, idx: u16, val: u16, info: &SliceInfo, sp: u32) -> TcResult<()> {
        match &info.source {
            SliceSource::Array => {
                let repr = self.ctx.types.repr_of(info.elem);
                let val = self.clone_arg(val, info.elem, sp);
                self.emit(Op::ArrSet { arr: recv, idx, val, repr }, sp);
                Ok(())
            }
            SliceSource::Str | SliceSource::Bytes => {
                self.ctx.err(Span::new(sp, sp + 1), "`str`/`bytes` are immutable — element assignment is not allowed");
                Err(())
            }
            SliceSource::DataBuf { buf_field, .. } => {
                let buf_ty = self.ctx.mk_array(info.elem);
                let buf = self.new_reg(buf_ty);
                self.emit(Op::GetF { dst: buf, obj: recv, field: *buf_field, repr: Repr::Ref }, sp);
                let repr = self.ctx.types.repr_of(info.elem);
                let val = self.clone_arg(val, info.elem, sp);
                self.emit(Op::ArrSet { arr: buf, idx, val, repr }, sp);
                Ok(())
            }
        }
    }
}
