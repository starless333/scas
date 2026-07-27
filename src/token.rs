#[derive(Debug, Clone, PartialEq)]
pub struct Token {
        token_type   : TokenType,
        lexeme       : String,
        literal      : Option<Literal>,
        line         : usize,
}

