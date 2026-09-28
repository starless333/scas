use crate::lexer::error::ScanError;
use crate::lexer::literal::Literal;
use crate::lexer::symbol::{Symbol, SymbolTable};
use crate::lexer::token::Token;
use crate::lexer::token_type::TokenType;
use std::collections::HashMap;
use std::sync::OnceLock;

pub struct Scanner {
    source: Vec<char>, // source code
    tokens: Vec<Token>,
    symbols: SymbolTable, // interns every token's lexeme
    start: usize,         // 0
    current: usize,       // 0
    line: usize,          // line starts at 1
    line_start: usize,
    token_line: usize,
    token_column: usize,
    errors: Vec<ScanError>,
}

static KEYWORDS_CELL: OnceLock<HashMap<&'static str, TokenType>> = OnceLock::new();

fn keywords() -> &'static HashMap<&'static str, TokenType> {
    KEYWORDS_CELL.get_or_init(|| {
        HashMap::from([
            ("and", TokenType::And),
            ("else", TokenType::Else),
            ("false", TokenType::False),
            ("for", TokenType::For),
            ("fn", TokenType::Fn),
            ("if", TokenType::If),
            ("nil", TokenType::Nil),
            ("or", TokenType::Or),
            ("print", TokenType::Print),
            ("return", TokenType::Return),
            ("true", TokenType::True),
            ("let", TokenType::Let),
            ("while", TokenType::While),
            ("exit", TokenType::Exit),
            ("help", TokenType::Help),
            ("clear", TokenType::Clear),
            ("break", TokenType::Break),
            ("continue", TokenType::Continue),
            ("xor", TokenType::Xor),
        ])
    })
}

impl Scanner {
    pub fn new(input: &str) -> Self {
        Self {
            source: input.chars().collect(),
            tokens: Vec::new(),
            symbols: SymbolTable::new(),
            start: 0,
            current: 0,
            line: 1,
            line_start: 0,
            token_line: 1,
            token_column: 1,
            errors: Vec::new(),
        }
    }

    /// Scan using an existing symbol table rather than starting a fresh
    /// one — e.g. a REPL that wants identifier symbols to stay stable
    /// across successive lines of input.
    pub fn with_symbols(input: &str, symbols: SymbolTable) -> Self {
        Self {
            source: input.chars().collect(),
            tokens: Vec::new(),
            symbols,
            start: 0,
            current: 0,
            line: 1,
            line_start: 0,
            token_line: 1,
            token_column: 1,
            errors: Vec::new(),
        }
    }

    // scans tokens of source input
    pub fn scan_tokens(&mut self) -> &Vec<Token> {
        while !self.is_end() {
            self.start = self.current;
            self.token_line = self.line;
            self.token_column = self.current - self.line_start + 1;
            self.scan_token();
        }
        let eof_symbol = self.symbols.intern("");
        self.tokens.push(Token {
            token_type: TokenType::Eof,
            symbol: eof_symbol,
            literal: None,
            line: self.line,
            column: self.current - self.line_start + 1,
            length: 0,
        });
        &self.tokens
    }

    pub fn had_error(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn errors(&self) -> &[ScanError] {
        &self.errors
    }

    /// Read-only access to the table while still scanning (e.g. to
    /// resolve a symbol for an error message).
    pub fn symbols(&self) -> &SymbolTable {
        &self.symbols
    }

    /// Hand back ownership of the interned symbol table once scanning
    /// is done, so the parser (or a later REPL line, via
    /// `with_symbols`) can keep using the same interning.
    pub fn into_symbols(self) -> SymbolTable {
        self.symbols
    }

    fn is_end(&self) -> bool {
        self.current >= self.source.len()
    }

    fn scan_token(&mut self) {
        let c = self.advance();
        match c {
            '(' => self.add_token(TokenType::LeftParen),
            ')' => self.add_token(TokenType::RightParen),
            '{' => self.add_token(TokenType::LeftBrace),
            '}' => self.add_token(TokenType::RightBrace),
            '[' => self.add_token(TokenType::LeftBracket),
            ']' => self.add_token(TokenType::RightBracket),
            ';' => self.add_token(TokenType::Semicolon),
            ',' => self.add_token(TokenType::Comma),
            '.' => self.add_token(TokenType::Dot),
            ':' => self.add_token(TokenType::Colon),
            '=' => {
                if self.check('=') {
                    self.add_token(TokenType::EqualEqual);
                } else {
                    self.add_token(TokenType::Equal);
                }
            }
            '!' => {
                if self.check('=') {
                    self.add_token(TokenType::BangEqual);
                } else {
                    self.add_token(TokenType::Bang);
                }
            }
            '+' => {
                if self.check('=') {
                    self.add_token(TokenType::PlusEqual);
                } else {
                    self.add_token(TokenType::Plus);
                }
            }
            '-' => {
                if self.check('=') {
                    self.add_token(TokenType::MinusEqual);
                } else {
                    self.add_token(TokenType::Minus);
                }
            }
            '*' => {
                if self.check('=') {
                    self.add_token(TokenType::StarEqual);
                } else {
                    self.add_token(TokenType::Star);
                }
            }
            '/' => {
                if self.check('=') {
                    self.add_token(TokenType::SlashEqual);
                } else if self.check('/') {
                    // comment case
                    while self.peek() != '\n' && !self.is_end() {
                        self.advance();
                    }
                } else {
                    self.add_token(TokenType::Slash);
                }
            }
            '%' => {
                if self.check('=') {
                    self.add_token(TokenType::ModEqual);
                } else {
                    self.add_token(TokenType::Mod);
                }
            }
            '^' => {
                if self.check('=') {
                    self.add_token(TokenType::PowEqual);
                } else {
                    self.add_token(TokenType::Pow);
                }
            }
            '&' => {
                if self.check('=') {
                    self.add_token(TokenType::AmpersandEqual);
                } else {
                    self.add_token(TokenType::Ampersand);
                }
            }
            '|' => {
                if self.check('=') {
                    self.add_token(TokenType::PipeEqual);
                } else {
                    self.add_token(TokenType::Pipe);
                }
            }
            '~' => {
                if self.check('=') {
                    self.add_token(TokenType::TildeEqual);
                } else {
                    self.add_token(TokenType::Tilde);
                }
            }
            '\n' => {
                self.line += 1;
                self.line_start = self.current;
            }
            ' ' => {}
            '\r' => {}
            '\t' => {}
            '"' => {
                self.string();
            }
            '<' => {
                if self.check('=') {
                    self.add_token(TokenType::LessEqual);
                } else if self.check('<') {
                    if self.check('=') {
                        self.add_token(TokenType::ShiftLeftEqual);
                    } else {
                        self.add_token(TokenType::ShiftLeft);
                    }
                } else {
                    self.add_token(TokenType::Less);
                }
            }
            '>' => {
                if self.check('=') {
                    self.add_token(TokenType::GreaterEqual);
                } else if self.check('>') {
                    if self.check('=') {
                        self.add_token(TokenType::ShiftRightEqual);
                    } else {
                        self.add_token(TokenType::ShiftRight);
                    }
                } else {
                    self.add_token(TokenType::Greater);
                }
            }
            '0' => {
                if self.peek() == 'b' || self.peek() == 'x' || self.peek() == 'o' {
                    self.bin_number();
                } else {
                    self.number();
                }
            }
            '∘' => {
                self.add_token(TokenType::FlowOperator);
            }
            _ => {
                if Self::is_digit(c) {
                    self.number();
                } else if Self::is_alpha(c) {
                    self.identifier();
                } else {
                    self.errors.push(ScanError::new(
                        format!("unexpected character `{c}`"),
                        self.token_line,
                        self.token_column,
                        1,
                    ));
                }
            }
        }
    }

    fn identifier(&mut self) {
        while Self::is_alpha_numeric(self.peek()) || Self::is_greek(self.peek()) {
            self.advance();
        }
        // Only needed to look the lexeme up in the keyword table --
        // for a plain identifier, the name itself is *not*
        // duplicated into `literal`; it's recovered later via
        // `symbols.resolve(token.symbol)`, since that's exactly
        // what interning already stored.
        let text: String = self.source[self.start..self.current].iter().collect();
        match keywords().get(text.as_str()) {
            Some(kw) => self.add_token(*kw),
            None => self.add_token(TokenType::Identifier),
        };
    }

    fn is_greek(c: char) -> bool {
        matches! (c, '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}')
    }

    // handles 0b / 0x / 0o prefixed integer literals (the leading '0' has
    // already been consumed by scan_token when this is called).
    fn bin_number(&mut self) {
        self.advance(); // consume 'b' / 'x' / 'o'
        while Self::is_alpha_numeric(self.peek()) {
            self.advance();
        }
        // The CAS needs the *exact* source text to parse this
        // number without precision loss later (arbitrary-precision
        // int/rational parsing happens downstream, not here) -- so
        // it's stored directly in `literal`, not just left to be
        // recovered via the symbol table. `add_token2` will also
        // intern this same span into `symbol` like every other
        // token, but `literal` is what the number-parsing stage
        // should actually read from.
        let text: String = self.source[self.start..self.current].iter().collect();
        self.add_token2(TokenType::Number, Some(Literal::Str(text)));
    }

    fn number(&mut self) {
        while Self::is_digit(self.peek()) {
            self.advance();
        }

        if self.peek() == '.' && Self::is_digit(self.npeek(2)) {
            self.advance();
            while Self::is_digit(self.peek()) {
                self.advance();
            }
        }

        if self.peek() == 'E' || self.peek() == 'e' {
            self.advance();
            if self.peek() == '-' || self.peek() == '+' {
                self.advance();
            }
            if !Self::is_digit(self.peek()) {
                self.errors.push(ScanError::new(
                    "expected digits after numeric exponent",
                    self.line,
                    self.current - self.line_start + 1,
                    1,
                ));
            } else {
                while Self::is_digit(self.peek()) {
                    self.advance();
                }
            }
        }

        // Same reasoning as `bin_number`: raw text preserved
        // verbatim in `literal` so nothing downstream has to
        // round-trip through the symbol table to parse it exactly.
        let text: String = self.source[self.start..self.current].iter().collect();
        self.add_token2(TokenType::Number, Some(Literal::Str(text)));
    }

    fn string(&mut self) {
        while self.peek() != '"' && !self.is_end() {
            if self.peek() == '\n' {
                self.advance();
                self.line += 1;
                self.line_start = self.current;
                continue;
            }
            self.advance();
        }

        if self.is_end() {
            self.errors.push(ScanError::new(
                "unterminated string",
                self.token_line,
                self.token_column,
                1,
            ));
            return;
        }
        self.advance(); // consume the closing '"'
                        // `symbol` (interned by add_token2) covers the full lexeme
                        // including the quotes; `literal` holds the unquoted
                        // contents, which is what an evaluator actually wants.
        let val: String = self.source[(self.start + 1)..(self.current - 1)]
            .iter()
            .collect();
        self.add_token2(TokenType::Str, Some(Literal::Str(val)));
    }

    fn is_digit(c: char) -> bool {
        c.is_ascii_digit()
    }

    fn is_alpha(c: char) -> bool {
        c.is_ascii_alphabetic() || c == '_'
    }

    fn is_alpha_numeric(c: char) -> bool {
        Self::is_digit(c) || Self::is_alpha(c)
    }

    // move up one char
    // requires : current is in a valid domain.
    fn advance(&mut self) -> char {
        let c = self.source[self.current];
        self.current += 1;
        c
    }

    fn add_token(&mut self, token: TokenType) {
        self.add_token2(token, None);
    }

    // more general add_token function -- interns the lexeme into the
    // symbol table instead of storing it as an owned String per token.
    fn add_token2(&mut self, token: TokenType, literal: Option<Literal>) {
        let lexeme: String = self.source[self.start..self.current].iter().collect();
        let symbol: Symbol = self.symbols.intern(&lexeme);
        self.tokens.push(Token {
            token_type: token,
            symbol,
            literal,
            line: self.token_line,
            column: self.token_column,
            length: self.current - self.start,
        });
    }

    // checks if next character matches expected, and if it does advances, else returns false.
    fn check(&mut self, c: char) -> bool {
        if self.is_end() {
            return false;
        }
        if self.source[self.current] != c {
            return false;
        }
        self.current += 1;
        true
    }

    // looks one character ahead.
    fn peek(&self) -> char {
        if self.is_end() {
            '\0'
        } else {
            self.source[self.current]
        }
    }

    // looks n characters ahead. n = 1 corresponds to normal peek
    fn npeek(&self, n: usize) -> char {
        if self.current + n > self.source.len() {
            '\0'
        } else {
            self.source[self.current + n - 1]
        }
    }
}
