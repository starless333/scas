use crate::lexer::token_type::TokenType;
use crate::lexer::literal::Literal;
use crate::lexer::symbol::Symbol;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
        pub token_type : TokenType,
        pub symbol     : Symbol,
        pub literal    : Option<Literal>,
        pub line       : usize,
}
