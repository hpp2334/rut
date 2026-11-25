//! `enum` — the member list is SLOTS: members are real nodes with
//! their own spans (`EnumMemberNode`), and a broken member region is a
//! `MemberRef::Error` beside the survivors (the uniform recovery law).

use crate::ast::doc::DocData;
use crate::ast::error::ErrorNode;
use crate::ast::handle::{AnyHandle, Handle};
use crate::ast::item::class::{item_vis, ModifierNode};
use crate::ast::item::ItemRef;
use crate::slot_alloc;
use crate::Name;
use rut_lexer::span::Span;

/// `pub? enum Name { A, B, .. }`
#[derive(Clone, Debug)]
pub struct EnumNode {
    /// starts at the item's FIRST token — the leading modifier when
    /// one is spelled, `enum` otherwise
    pub span: Span,
    /// the `{..}` region, braces included — the head slice's right cut
    pub body_span: Span,
    /// did the frame eat the member list's own `}`? `false` when the
    /// parse closed the list at EOF — the enum is still open (the
    /// edit lane's survivor rule asks exactly this)
    pub closed: bool,
    /// the modifier chain in source order — `vis()` derives from it
    pub modifiers: Vec<ModifierNode>,
    pub name: Name,
    pub members: Vec<MemberRef>,
    pub doc: DocData,
}

impl EnumNode {
    /// the spelled item visibility — `Vis::Self_` when unannotated
    pub fn vis(&self) -> crate::ast::item::Vis {
        item_vis(&self.modifiers)
    }
}

/// one enum member — a node since v2 (its name is recorded at parse;
/// the ordered-token-matching recovery is gone).
#[derive(Clone, Debug)]
pub struct EnumMemberNode {
    pub span: Span,
    pub name: Name,
    pub doc: DocData,
}

/// the member slot — a member or an ErrorNode in its place
#[derive(Clone, Copy, Debug)]
pub enum MemberRef {
    Member(Handle<EnumMemberNode>),
    Error(Handle<ErrorNode>),
}

impl MemberRef {
    /// the universal address — the tree index's walk currency
    pub fn any(self) -> AnyHandle {
        crate::slot_any!(self, MemberRef::Member, MemberRef::Error)
    }
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
