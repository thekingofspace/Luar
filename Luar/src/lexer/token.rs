
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {

    Identifier,

    Int,

    Float,

    Str,

    InterpStr,

    Operator,

    Delimiter,

    Comment,

    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {

    pub start: usize,

    pub end: usize,

    pub line: u32,

    pub col: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,

    pub text: String,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, text: impl Into<String>, span: Span) -> Self {
        Token { kind, text: text.into(), span }
    }
}
