// example use of scas.
// f(x) = x % 5
// y = 10
// d/dx(f) or d/dx(f(x))
// int(f,0,1)
// sum(f,0,10) == sum_{x=0}^{10} f(x)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenType {
        // Single-character punctuation
        LEFT_PAREN, RIGHT_PAREN, // done
        LEFT_BRACKET, RIGHT_BRACKET, // matrices?
        LEFT_BRACE, RIGHT_BRACE, // done
        COMMA, // done
        DOT, // done
        COLON, // done
        SEMICOLON, // done

        // Arithmetic
        PLUS, // done
        MINUS, // done
        STAR, // done
        SLASH, // done
        MOD, // done
        POW, // done

        // Bitwise
        AMPERSAND, // done
        PIPE, // done
        XOR,
        TILDE, // done
        SHIFT_LEFT,
        SHIFT_RIGHT,
        SHIFT_LEFT_EQUAL,
        SHIFT_RIGHT_EQUAL,

        // Assignment
        EQUAL, // done
        PLUS_EQUAL, // done
        MINUS_EQUAL, // done
        STAR_EQUAL, // done
        SLASH_EQUAL, // done
        MOD_EQUAL, // done
        POW_EQUAL, // done
        AMPERSAND_EQUAL, // done
        PIPE_EQUAL, // done
        TILDE_EQUAL, // done

        // Comparison
        EQUAL_EQUAL, // done
        BANG_EQUAL, // done
        LESS,
        LESS_EQUAL,
        GREATER,
        GREATER_EQUAL,

        // Logical
        BANG, // done
        AND, // done
        OR, // done

        // Literals
        IDENTIFIER,
        NUMBER, // done
        STRING, // done

        // Keywords
        LET, // done
        FN, // done
        IF, // done
        ELSE, // done
        FOR, // done
        WHILE, // done
        BREAK, // done
        CONTINUE, // done
        RETURN, // done
        PRINT, // done

        TRUE, // done
        FALSE, // done
        NIL, // done

        // REPL
        EXIT, // done
        HELP, // done
        CLEAR, // done

        EOF,
}
