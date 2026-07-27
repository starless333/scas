use crate::token_type::TokenType;
use crate::literal::Literal;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
        pub token_type : TokenType,
        pub lexeme     : String,
        pub literal    : Option<Literal>,
        pub line       : usize,
}
