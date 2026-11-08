//! `enum` — the member list is SLOTS: members are real nodes with
//! their own spans (`EnumMemberNode`), and a broken member region is a
//! `MemberRef::Error` beside the survivors (the uniform recovery law).

use crate::doc::DocData;
use crate::error::ErrorNode;
use crate::handle::Handle;
use crate::item::ItemRef;
use crate::slot_alloc;
use rut_core::IdentId;
use rut_lexer::span::Span;

/// `pub? enum Name { A, B, .. }`
#[derive(Clone, Debug)]
pub struct EnumNode {
    pub span: Span,
    pub vis: crate::item::Vis,
    pub name: IdentId,
    pub name_span: Span,
    pub members: Vec<MemberRef>,
    pub doc: DocData,
}

/// one enum member — a node since v2 (its `name_span` is recorded at
/// parse; the ordered-token-matching recovery is gone).
#[derive(Clone, Debug)]
pub struct EnumMemberNode {
    pub span: Span,
    pub name: IdentId,
    pub name_span: Span,
    pub doc: DocData,
}

/// the member slot — a member or an ErrorNode in its place
#[derive(Clone, Copy, Debug)]
pub enum MemberRef {
    Member(Handle<EnumMemberNode>),
    Error(Handle<ErrorNode>),
}

impl MemberRef {
    pub fn span(self, ctx: &crate::Ast) -> Span {
        match self {
            MemberRef::Member(h) => h.span(ctx),
            MemberRef::Error(h) => h.span(ctx),
        }
    }
    pub fn is_error(self) -> bool {
        matches!(self, MemberRef::Error(_))
    }
    pub fn as_member(self) -> Option<Handle<EnumMemberNode>> {
        match self {
            MemberRef::Member(h) => Some(h),
            MemberRef::Error(_) => None,
        }
    }
}

slot_alloc!(EnumMemberNode, MemberRef::Member);
slot_alloc!(EnumNode, ItemRef::Enum);
