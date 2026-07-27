mod error_handling;
mod token_type;
mod token;
use error_handling::*;
use std::collections::HashMap;

pub struct Scanner {
        source    : Vec<char>, // source code
        tokens    : Vec<Token>,
        start     : usize, // 0
        current   : usize, // 0
        line      : usize, // line starts at 1
        has_error : bool,
}

const keywords : HashMap = HashMap::from ([
        ("and", TokenType::AND),
        ("else",   TokenType::ELSE),
        ("false",  TokenType::FALSE),
        ("for",    TokenType::FOR),
        ("fn",    TokenType::FN),
        ("if",     TokenType::IF),
        ("nil",    TokenType::NIL),
        ("or",     TokenType::OR),
        ("print",  TokenType::PRINT),
        ("return", TokenType::RETURN),
        ("true",   TokenType::TRUE),
        ("let",    TokenType::LET),
        ("while",  TokenType::WHILE),
        ("nil",    TokenType::NIL),
        ("exit",   TokenType::EXIT),
        ("help",   TokenType::HELP),
        ("clear",  TokenType::CLEAR),
        ("break",  TokenType::BREAK),
        ("continue",  TokenType::CONTINUE),
]);

impl Scanner {
        pub fn new(input: &str) -> Self {
                Self {
                        source: input.chars().collect(),
                        tokens: Vec::new(),
                        start: 0,
                        current: 0,
                        line: 1,
                        has_error: false,
                }
        }
        
        // scans tokens of source input
        pub fn scan_tokens (&mut self) {
                while !self.is_end () {
                        self.start = self.current;
                        self.scan_token ();
                }
                self.tokens.push ( Token {
                        token_type : TokenType::EOF,
                        lexeme     : String::from (""),
                        literal    : None,
                        line       : self.line,
                } );
        }

        // TODO : make sure that we gracefully crash if the user throws in nasty chars.
        fn is_end (&self) -> bool {
                return self.current >= self.source.len() as i32;
        }

        fn scan_token (&mut self) {
                let c = self.advance ();
                match c {
                        '(' => self.add_token (TokenType::LEFT_PAREN),
                        ')' => self.add_token (TokenType::RIGHT_PAREN),
                        '{' => self.add_token (TokenType::LEFT_BRACE),
                        '}' => self.add_token (TokenType::RIGHT_BRACE),
                        ';' => self.add_token (TokenType::SEMICOLON),
                        ',' => self.add_token (TokenType::COMMA),
                        '.' => self.add_token (TokenType::DOT),
                        ':' => self.add_token (TokenType::COLON),
                        '=' => {
                                if check ('=') {
                                        self.add_token (TokenType::EQUAL_EQUAL);
                                } else {
                                        self.add_token (TokenType::EQUAL);
                                }
                        },
                        '!' => {
                                if check ('=') {
                                        self.add_token (TokenType::BANG_EQUAL);
                                } else {
                                        self.add_token (TokenType::BANG);
                                }
                        },
                        '+' => {
                                if check ('=') {
                                        self.add_token (TokenType::PLUS_EQUAL);
                                } else {
                                        self.add_token (TokenType::PLUS);
                                }
                        },
                        '-' => {
                                if check ('=') {
                                        self.add_token (TokenType::MINUS_EQUAL);
                                } else {
                                        self.add_token (TokenType::MINUS);
                                }
                        },
                        '*' => {
                                if check ('=') {
                                        self.add_token (TokenType::STAR_EQUAL);
                                } else {
                                        self.add_token (TokenType::STAR);
                                }
                        },
                        '/' => {
                                if check ('=') {
                                        self.add_token (TokenType::SLASH_EQUAL);
                                } else if check ('/') { // comment case
                                        while self.peek () != '\n' && !self.is_end() {
                                                self.advance ();
                                        }
                                } else {
                                        self.add_token (TokenType::SLASH);
                                }
                        },
                        '%' => {
                                if check ('=') {
                                        self.add_token (TokenType::MOD_EQUAL);
                                } else {
                                        self.add_token (TokenType::MOD);
                                }
                        },
                        '^' => {
                                if check ('=') {
                                        self.add_token (TokenType::POW_EQUAL);
                                } else {
                                        self.add_token (TokenType::POW);
                                }
                        },
                        '&' => {
                                if check ('=') {
                                        self.add_token (TokenType::AMPERSAND_EQUAL);
                                } else {
                                        self.add_token (TokenType::AMPERSAND);
                                }
                        },
                        '|' => {
                                if check ('=') {
                                        self.add_token (TokenType::PIPE_EQUAL);
                                } else {
                                        self.add_token (TokenType::PIPE);
                                }
                        },
                        '~' => {
                                if check ('=') {
                                        self.add_token (TokenType::TILDE_EQUAL);
                                } else {
                                        self.add_token (TokenType::TILDE);
                                }
                        },
                        '\n' => {
                                self.line += 1;
                        },
                        ' '  => {},
                        '\r' => {},
                        '\t' => {},
                        '"'  => { self.string (); },
                        '0'  => {
                                if is_digit (self.peek()) || self.peek () == '.' {
                                        self.number ();
                                } else {
                                        self.bin_number();
                                }
                        },
                        '<' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::LESS_EQUAL);
                                } else {
                                        if self.check ('<') {
                                                if self.check ('=') {
                                                        self.add_token (TokenType::SHIFT_LEFT_EQUAL);
                                                } else {
                                                        self.add_token (TokenType::SHIFT_LEFT);
                                                }
                                        } else {
                                                self.add_token (TokenType::LESS);
                                        }
                                }
                        },
                        '>' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::GREATER_EQUAL);
                                } else {
                                        if self.check ('>') {
                                                if self.check ('=') {
                                                        self.add_token (TokenType::SHIFT_RIGHT_EQUAL);
                                                } else {
                                                        self.add_token (TokenType::SHIFT_RIGHT);
                                                }
                                        } else {
                                                self.add_token (TokenType::GREATER);
                                        }
                                }
                        },
                        _  => {
                                if is_digit (c) {
                                        self.number ();
                                } else if is_alpha (c) {
                                        self.identifier ();
                                }else {
                                        error (self.line, "unexpected character.");
                                        self.has_error = true;
                                }
                        }
                }
        }

        fn identifier (&mut self) {
                while is_alpha_numeric (self.peek ()) || is_greek (self.peek ())  { self.advance (); }
                let val = self.source[self.start..self.current].to_string ();
                match keywords.get (val) {
                        Some (vl) => self.add_token  (vl),
                        None      => self.add_token2 (TokenType::IDENTIFIER, val),
                };
        }

        fn is_greek (c : char) -> bool {
                matches!(c, '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}')
        }

        fn bin_number (&mut self) {
                let c = self.peek ();
                self.advance();
                if c == 'b' || c == 'x' || c == 'o' {
                        while is_digit (self.peek()) { self.advance (); }
                        self.add_token2 (TokenType::NUMBER,self.source[self.start..self.current].to_string());
                } else {
                        error (self.line, "invalid number representation");
                }
        }
        
        fn number (&mut self) {
                while is_digit (self.peek ()) { self.advance ();}
                
                
                if self.peek () == '.' && is_digit (self.peekn (2)) {
                        self.advance ();
                        while (is_digit (self.peek ())) { self.advance ();}
                }

                if self.peek () == 'E' || self.peek () == 'e' {
                        self.advance ();
                        if is_digit (self.peek()) || self.peek() == '-' {
                                self.advance ();
                                while is_digit (self.peek ()) { self.advance ();}
                        }
                }
                
                self.add_token2 (TokenType::NUMBER,self.source[self.start..self.current].to_string());
        }
        
        fn string (&mut self) {
                while self.peek () != '"' && !self.is_end () {
                        if self.peek () == '\n' {
                                self.line += 1;
                                self.advance ();
                        }
                }

                if self.is_end () {
                        error (self.line, "unterminated string");
                        return;
                }
                self.advance ();
                let val = self.source[(self.start + 1)..(self.current - 1)].to_string ();
                self.add_token2 (TokenType::STRING, val);
        }

        fn is_digit (c : char) -> bool {
                return c >= '0' && c <= '9';
        }

        fn is_alpha (c : char) -> bool {
                return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || c == '_';
        }

        fn is_alpha_numeric (c : char) -> bool {
                return is_digit (c) || is_alpha (c);
        }
        
        // move up one char
        // requires : current is in a valid domain.
        fn advance (&mut self) -> char {
                let c = self.source[self.current];
                self.current += 1;
                return c;
        }

        fn add_token (&mut self, token : TokenType) {
                self.add_token2 (token, None);
        }

        // more general add_token function
        fn add_token2 (&mut self, token : TokenType, literal : Literal) {
                let lexeme = (&self.source[start..current]).to_string ();
                self.tokens.push ( Token {
                        token_type : token,
                        lexeme     : lexeme,
                        literal    : literal,
                        line       : self.line,
                } );
        }

        // checks if next character matches expected, and if it does advances, else it returns false.
        fn check (&mut self, c : char) -> bool {
                if self.is_end () {
                        return false;
                }
                if self.source[self.current] != c {
                        return false;
                }
                self.current += 1;
                return true;
        }

        // looks one character ahead.
        fn peek (&self) -> char {
                if self.is_end () {
                        return '\0';
                } else {
                        return self.source[self.current];
                }
        }

        // looks n characters ahead. n = 1 corresponds to normal peek
        fn peekn (&self, n : usize) -> char {
                if self.current + n - 1 >= self.source.len () {
                        return '\0';
                } else {
                        return self.source[current + n - 1];
                }
        }
        
}
