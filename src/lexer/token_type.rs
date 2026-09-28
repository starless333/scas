// example use of scas.
// f(x) = x % 5
// y = 10
// d/dx(f) or d/dx(f(x))
// int(f,0,1)
// sum(f,0,10) == sum_{x=0}^{10} f(x)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenType {
    // Single-character punctuation
    LeftParen,
    RightParen, // done
    LeftBracket,
    RightBracket, // matrices?
    LeftBrace,
    RightBrace, // done
    Comma,      // done
    Dot,        // done
    Colon,      // done
    Semicolon,  // done
    FlowOperator,

    // Arithmetic
    Plus,  // done
    Minus, // done
    Star,  // done
    Slash, // done
    Mod,   // done
    Pow,   // done

    // Bitwise
    Ampersand, // done
    Pipe,      // done
    Xor,
    Tilde, // done
    ShiftLeft,
    ShiftRight,
    ShiftLeftEqual,
    ShiftRightEqual,

    // Assignment
    Equal,          // done
    PlusEqual,      // done
    MinusEqual,     // done
    StarEqual,      // done
    SlashEqual,     // done
    ModEqual,       // done
    PowEqual,       // done
    AmpersandEqual, // done
    PipeEqual,      // done
    TildeEqual,     // done

    // Comparison
    EqualEqual, // done
    BangEqual,  // done
    Less,
    LessEqual,
    Greater,
    GreaterEqual,

    // Logical
    Bang, // done
    And,  // done
    Or,   // done

    // Literals
    Identifier,
    Number, // done
    Str,    // done

    // Keywords
    Let,      // done
    Fn,       // done
    If,       // done
    Else,     // done
    For,      // done
    While,    // done
    Break,    // done
    Continue, // done
    Return,   // done
    Print,    // done

    True,  // done
    False, // done
    Nil,   // done

    // REPL
    Exit,  // done
    Help,  // done
    Clear, // done

    Eof,
}
