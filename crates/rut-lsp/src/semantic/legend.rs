//! The legend — semantic token types in legend order, plus the word
//! classes the token layer owns by text.

/// Semantic token types, in legend order. **Standard types only** — stock
/// themes color them in every LSP editor without scope mappings (the VS
/// Code extension still ships `semanticTokenScopes` as belt & braces).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenType {
    Keyword,
    Number,
    String,
    Type,
    Function,
    Method,
    Variable,
    Parameter,
    Property,
    Enum,
    EnumMember,
    Class,
    Interface,
    Operator,
}

pub const ALL: [TokenType; 14] = [
    TokenType::Keyword,
    TokenType::Number,
    TokenType::String,
    TokenType::Type,
    TokenType::Function,
    TokenType::Method,
    TokenType::Variable,
    TokenType::Parameter,
    TokenType::Property,
    TokenType::Enum,
    TokenType::EnumMember,
    TokenType::Class,
    TokenType::Interface,
    TokenType::Operator,
];

impl TokenType {
    pub fn name(self) -> &'static str {
        match self {
            TokenType::Keyword => "keyword",
            TokenType::Number => "number",
            TokenType::String => "string",
            TokenType::Type => "type",
            TokenType::Function => "function",
            TokenType::Method => "method",
            TokenType::Variable => "variable",
            TokenType::Parameter => "parameter",
            TokenType::Property => "property",
            TokenType::Enum => "enum",
            TokenType::EnumMember => "enumMember",
            TokenType::Class => "class",
            TokenType::Interface => "interface",
            TokenType::Operator => "operator",
        }
    }
    pub fn index(self) -> u32 {
        ALL.iter().position(|t| *t == self).unwrap() as u32
    }
    pub fn legend() -> Vec<&'static str> {
        ALL.iter().map(|t| t.name()).collect()
    }
}

/// The highlighting keyword set: the grammar's reserved words
/// (`rut_parser::is_reserved_kw`) plus the contextual/statement words the
/// parser matches by text (`break`, `continue`, `as`, `super`, `default`)
/// and the receiver `self`. `true`/`false` lex as `Tok::Bool` and are
/// classified there (keyword constants).
pub fn is_keyword(s: &str) -> bool {
    matches!(s, "self" | "break" | "continue" | "as" | "super" | "default")
        || rut_parser::is_reserved_kw(s)
}

pub fn is_primitive_ty(s: &str) -> bool {
    // canonical table lives with the grammar (shared with `host primitive`)
    rut_parser::is_primitive_ty(s)
}

/// A word the token layer already owns by text (keywords, `Self`) — never
/// a name position. Primitives (`unit`, `str`, …) are contextual type
/// names, NOT reserved: a field or param may legally carry one, and the
/// AST pass then overrides the token layer's `type` class.
pub(crate) fn owned_by_tokens(s: &str) -> bool {
    is_keyword(s) || s == "Self"
}

pub(crate) fn is_cap(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_uppercase())
}
