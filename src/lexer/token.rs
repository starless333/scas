use crate::lexer::literal::Literal;
use crate::lexer::symbol::Symbol;
use crate::lexer::token_type::TokenType;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub token_type: TokenType,
    pub symbol: Symbol,
    pub literal: Option<Literal>,
    pub line: usize,
    pub column: usize,
    /// Length of the token in Unicode scalar values. EOF has length zero.
    pub length: usize,
    /// Half-open UTF-8 byte range in the original source.
    pub byte_start: usize,
    pub byte_end: usize,
}
