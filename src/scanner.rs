use crate::error_handling::*;
use crate::token::Token;
use crate::token_type::TokenType;
use crate::literal::Literal;
use std::collections::HashMap;
use std::sync::OnceLock;

pub struct Scanner {
        source    : Vec<char>, // source code
        tokens    : Vec<Token>,
        start     : usize, // 0
        current   : usize, // 0
        line      : usize, // line starts at 1
        has_error : bool,
}

static KEYWORDS_CELL : OnceLock<HashMap<&'static str, TokenType>> = OnceLock::new ();

fn keywords () -> &'static HashMap<&'static str, TokenType> {
        KEYWORDS_CELL.get_or_init (|| {
                HashMap::from ([
                        ("and",      TokenType::AND),
                        ("else",     TokenType::ELSE),
                        ("false",    TokenType::FALSE),
                        ("for",      TokenType::FOR),
                        ("fn",       TokenType::FN),
                        ("if",       TokenType::IF),
                        ("nil",      TokenType::NIL),
                        ("or",       TokenType::OR),
                        ("print",    TokenType::PRINT),
                        ("return",   TokenType::RETURN),
                        ("true",     TokenType::TRUE),
                        ("let",      TokenType::LET),
                        ("while",    TokenType::WHILE),
                        ("exit",     TokenType::EXIT),
                        ("help",     TokenType::HELP),
                        ("clear",    TokenType::CLEAR),
                        ("break",    TokenType::BREAK),
                        ("continue", TokenType::CONTINUE),
                        ("xor"     , TokenType::XOR),
                ])
        })
}

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
        pub fn scan_tokens (&mut self) -> &Vec<Token> {
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
                &self.tokens
        }

        pub fn had_error (&self) -> bool {
                self.has_error
        }

        fn is_end (&self) -> bool {
                self.current >= self.source.len ()
        }

        fn scan_token (&mut self) {
                let c = self.advance ();
                match c {
                        '(' => self.add_token (TokenType::LEFT_PAREN),
                        ')' => self.add_token (TokenType::RIGHT_PAREN),
                        '{' => self.add_token (TokenType::LEFT_BRACE),
                        '}' => self.add_token (TokenType::RIGHT_BRACE),
                        '[' => self.add_token (TokenType::LEFT_BRACKET),
                        ']' => self.add_token (TokenType::RIGHT_BRACKET),
                        ';' => self.add_token (TokenType::SEMICOLON),
                        ',' => self.add_token (TokenType::COMMA),
                        '.' => self.add_token (TokenType::DOT),
                        ':' => self.add_token (TokenType::COLON),
                        '=' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::EQUAL_EQUAL);
                                } else {
                                        self.add_token (TokenType::EQUAL);
                                }
                        },
                        '!' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::BANG_EQUAL);
                                } else {
                                        self.add_token (TokenType::BANG);
                                }
                        },
                        '+' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::PLUS_EQUAL);
                                } else {
                                        self.add_token (TokenType::PLUS);
                                }
                        },
                        '-' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::MINUS_EQUAL);
                                } else {
                                        self.add_token (TokenType::MINUS);
                                }
                        },
                        '*' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::STAR_EQUAL);
                                } else {
                                        self.add_token (TokenType::STAR);
                                }
                        },
                        '/' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::SLASH_EQUAL);
                                } else if self.check ('/') { // comment case
                                        while self.peek () != '\n' && !self.is_end () {
                                                self.advance ();
                                        }
                                } else {
                                        self.add_token (TokenType::SLASH);
                                }
                        },
                        '%' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::MOD_EQUAL);
                                } else {
                                        self.add_token (TokenType::MOD);
                                }
                        },
                        '^' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::POW_EQUAL);
                                } else {
                                        self.add_token (TokenType::POW);
                                }
                        },
                        '&' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::AMPERSAND_EQUAL);
                                } else {
                                        self.add_token (TokenType::AMPERSAND);
                                }
                        },
                        '|' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::PIPE_EQUAL);
                                } else {
                                        self.add_token (TokenType::PIPE);
                                }
                        },
                        '~' => {
                                if self.check ('=') {
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
                        '<' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::LESS_EQUAL);
                                } else if self.check ('<') {
                                        if self.check ('=') {
                                                self.add_token (TokenType::SHIFT_LEFT_EQUAL);
                                        } else {
                                                self.add_token (TokenType::SHIFT_LEFT);
                                        }
                                } else {
                                        self.add_token (TokenType::LESS);
                                }
                        },
                        '>' => {
                                if self.check ('=') {
                                        self.add_token (TokenType::GREATER_EQUAL);
                                } else if self.check ('>') {
                                        if self.check ('=') {
                                                self.add_token (TokenType::SHIFT_RIGHT_EQUAL);
                                        } else {
                                                self.add_token (TokenType::SHIFT_RIGHT);
                                        }
                                } else {
                                        self.add_token (TokenType::GREATER);
                                }
                        },
                        '0' => {
                                if self.peek () == 'b' || self.peek () == 'x' || self.peek () == 'o' {
                                        self.bin_number ();
                                } else if Self::is_digit (self.peek ()) || self.peek () == '.' {
                                        self.number ();
                                } else {
                                        self.add_token2 (TokenType::NUMBER, Some (Literal::INT (0)));
                                }
                        },
                        _  => {
                                if Self::is_digit (c) {
                                        self.number ();
                                } else if Self::is_alpha (c) {
                                        self.identifier ();
                                } else {
                                        error (self.line, "unexpected character.");
                                        self.has_error = true;
                                }
                        }
                }
        }

        fn identifier (&mut self) {
                while Self::is_alpha_numeric (self.peek ()) || Self::is_greek (self.peek ()) { self.advance (); }
                let val : String = self.source[self.start..self.current].iter ().collect ();
                match keywords ().get (val.as_str ()) {
                        Some (kw) => self.add_token (kw.clone ()),
                        None      => self.add_token2 (TokenType::IDENTIFIER, Some (Literal::STRING (val))),
                };
        }

        fn is_greek (c : char) -> bool {
                matches! (c, '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}')
        }

        // handles 0b / 0x / 0o prefixed integer literals (the leading '0' has
        // already been consumed by scan_token when this is called).
        fn bin_number (&mut self) {
                self.advance (); // consume 'b' / 'x' / 'o'
                while Self::is_alpha_numeric (self.peek ()) { self.advance (); }
                let text : String = self.source[self.start..self.current].iter ().collect ();
                self.add_token2 (TokenType::NUMBER, Some (Literal::STRING (text)));
        }

        fn number (&mut self) {
                while Self::is_digit (self.peek ()) { self.advance (); }

                if self.peek () == '.' && Self::is_digit (self.npeek (2)) {
                        self.advance ();
                        while Self::is_digit (self.peek ()) { self.advance (); }
                }

                if self.peek () == 'E' || self.peek () == 'e' {
                        self.advance ();
                        if Self::is_digit (self.peek ()) || self.peek () == '-' {
                                self.advance ();
                                while Self::is_digit (self.peek ()) { self.advance (); }
                        }
                }

                let text : String = self.source[self.start..self.current].iter ().collect ();
                self.add_token2 (TokenType::NUMBER, Some (Literal::STRING (text)));
        }

        fn string (&mut self) {
                while self.peek () != '"' && !self.is_end () {
                        if self.peek () == '\n' {
                                self.line += 1;
                        }
                        self.advance ();
                }

                if self.is_end () {
                        error (self.line, "unterminated string");
                        self.has_error = true;
                        return;
                }
                self.advance (); // consume the closing '"'
                let val : String = self.source[(self.start + 1)..(self.current - 1)].iter ().collect ();
                self.add_token2 (TokenType::STRING, Some (Literal::STRING (val)));
        }

        fn is_digit (c : char) -> bool {
                c >= '0' && c <= '9'
        }

        fn is_alpha (c : char) -> bool {
                (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || c == '_'
        }

        fn is_alpha_numeric (c : char) -> bool {
                Self::is_digit (c) || Self::is_alpha (c)
        }

        // move up one char
        // requires : current is in a valid domain.
        fn advance (&mut self) -> char {
                let c = self.source[self.current];
                self.current += 1;
                c
        }

        fn add_token (&mut self, token : TokenType) {
                self.add_token2 (token, None);
        }

        // more general add_token function
        fn add_token2 (&mut self, token : TokenType, literal : Option<Literal>) {
                let lexeme : String = self.source[self.start..self.current].iter ().collect ();
                self.tokens.push ( Token {
                        token_type : token,
                        lexeme     : lexeme,
                        literal    : literal,
                        line       : self.line,
                } );
        }

        // checks if next character matches expected, and if it does advances, else returns false.
        fn check (&mut self, c : char) -> bool {
                if self.is_end () {
                        return false;
                }
                if self.source[self.current] != c {
                        return false;
                }
                self.current += 1;
                true
        }

        // looks one character ahead.
        fn peek (&self) -> char {
                if self.is_end () {
                        '\0'
                } else {
                        self.source[self.current]
                }
        }

        // looks n characters ahead. n = 1 corresponds to normal peek
        fn npeek (&self, n : usize) -> char {
                if self.current + n - 1 >= self.source.len () {
                        '\0'
                } else {
                        self.source[self.current + n - 1]
                }
        }
}
