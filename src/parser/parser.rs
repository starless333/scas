use crate::ast::assign_op::*;
use crate::ast::binary_op::*;
use crate::ast::expr::*;
use crate::ast::statement::*;
use crate::ast::unary_op::*;
use crate::lexer::literal::Literal;
use crate::lexer::token::Token;
use crate::lexer::token_type::TokenType;

#[derive(Debug, Clone, PartialEq)]
pub struct Parser {
        tokens: Vec<Token>,
        current: usize,
}

impl Parser {
        pub fn new(tokens: Vec<Token>) -> Self {
                Parser { tokens, current: 0 }
        }

        /// Parses the entire program into a sequence of statements.
        pub fn parse(&mut self) -> Vec<Statement> {
                let mut statements = Vec::new();
                while !self.is_at_end() {
                        statements.push(self.statement());
                }
                statements
        }

        // ==========================================
        //                STATEMENTS
        // ==========================================

        fn statement(&mut self) -> Statement {
                if self.match_token(&[TokenType::Let]) {
                        return self.variable_declaration();
                }
                if self.match_token(&[TokenType::Fn]) {
                        return self.function_declaration();
                }
                if self.match_token(&[TokenType::If]) {
                        return self.if_statement();
                }
                if self.match_token(&[TokenType::While]) {
                        return self.while_statement();
                }
                if self.match_token(&[TokenType::For]) {
                        return self.for_statement();
                }
                if self.match_token(&[TokenType::Return]) {
                        return self.return_statement();
                }
                if self.match_token(&[TokenType::Break]) {
                        self.consume(TokenType::Semicolon, "Expected ';' after 'break'.");
                        return Statement::Break;
                }
                if self.match_token(&[TokenType::Continue]) {
                        self.consume(TokenType::Semicolon, "Expected ';' after 'continue'.");
                        return Statement::Continue;
                }
                if self.match_token(&[TokenType::Print]) {
                        return self.print_statement();
                }
                if self.match_token(&[TokenType::LeftBrace]) {
                        return Statement::Block(self.block());
                }

                self.expression_statement()
        }

        fn variable_declaration(&mut self) -> Statement {
                // `.symbol` is a `Copy` u32 id from the interner -- no
                // allocation, no need to touch the symbol table here.
                let name = self.consume(TokenType::Identifier, "Expected variable name.").symbol;

                let mut value = None;
                if self.match_token(&[TokenType::Equal]) {
                        value = Some(self.expression());
                }

                self.consume(TokenType::Semicolon, "Expected ';' after variable declaration.");
                Statement::Let { name, value }
        }

        fn function_declaration(&mut self) -> Statement {
                let name = self.consume(TokenType::Identifier, "Expected function name.").symbol;
                self.consume(TokenType::LeftParen, "Expected '(' after function name.");

                let mut params = Vec::new();
                if !self.check(TokenType::RightParen) {
                        loop {
                                params.push(self.consume(TokenType::Identifier, "Expected parameter name.").symbol);
                                if !self.match_token(&[TokenType::Comma]) {
                                        break;
                                }
                        }
                }
                self.consume(TokenType::RightParen, "Expected ')' after parameters.");
                self.consume(TokenType::Equal, "Expected '=' before function body.");

                let body = self.expression();
                self.consume(TokenType::Semicolon, "Expected ';' after function body.");
                Statement::Function { name, params, body }
        }

        fn if_statement(&mut self) -> Statement {
                let condition = self.expression();

                self.consume(TokenType::LeftBrace, "Expected '{' before 'if' body.");
                let then_branch = Box::new(Statement::Block(self.block()));

                let mut else_branch = None;
                if self.match_token(&[TokenType::Else]) {
                        self.consume(TokenType::LeftBrace, "Expected '{' before 'else' body.");
                        else_branch = Some(Box::new(Statement::Block(self.block())));
                }

                Statement::If { condition, then_branch, else_branch }
        }

        fn while_statement(&mut self) -> Statement {
                let condition = self.expression();
                self.consume(TokenType::LeftBrace, "Expected '{' before 'while' body.");
                let body = Box::new(Statement::Block(self.block()));

                Statement::While { condition, body }
        }

        fn for_statement(&mut self) -> Statement {
                let init = Box::new(if self.match_token(&[TokenType::Let]) {
                        self.variable_declaration() // This consumes its own ';'
                } else {
                        self.expression_statement() // This consumes its own ';'
                });

                let condition = self.expression();
                self.consume(TokenType::Semicolon, "Expected ';' after for loop condition.");

                // Parse the update (e.g., i += 1) WITHOUT expecting a semicolon afterwards
                let update = Box::new(self.parse_assignment_or_expr());

                self.consume(TokenType::LeftBrace, "Expected '{' before 'for' body.");
                let body = Box::new(Statement::Block(self.block()));

                Statement::For { init, condition, update, body }
        }

        fn return_statement(&mut self) -> Statement {
                let value = if !self.check(TokenType::Semicolon) {
                        Some(self.expression())
                } else {
                        None
                };
                self.consume(TokenType::Semicolon, "Expected ';' after return value.");
                Statement::Return(value)
        }

        fn print_statement(&mut self) -> Statement {
                self.consume(TokenType::LeftParen, "Expected '(' after print.");
                let value = self.expression();
                self.consume(TokenType::RightParen, "Expected ')' after print expression.");
                self.consume(TokenType::Semicolon, "Expected ';' after value.");
                Statement::Print(value)
        }

        fn expression_statement(&mut self) -> Statement {
                let stmt = self.parse_assignment_or_expr();
                self.consume(TokenType::Semicolon, "Expected ';' after statement.");
                stmt
        }

        /// Helper to parse either an assignment OR an expression, without consuming a trailing semicolon.
        /// This is crucial because `for` loop updates don't have semicolons, but normal statements do.
        fn parse_assignment_or_expr(&mut self) -> Statement {
                // Lookahead to check if this is an assignment statement (Identifier Operator Expr)
                if self.check(TokenType::Identifier) {
                        if let Some(next) = self.tokens.get(self.current + 1) {
                                if let Some(op) = self.get_assign_op(next.token_type) {
                                        let name = self.advance().symbol; // Consume Identifier
                                        self.advance(); // Consume Op
                                        let value = self.expression();
                                        return Statement::Assign { name, op, value };
                                }
                        }
                }

                // If it's not an assignment, it's just a normal expression
                Statement::Expr(self.expression())
        }

        fn block(&mut self) -> Vec<Statement> {
                let mut statements = Vec::new();
                while !self.check(TokenType::RightBrace) && !self.is_at_end() {
                        statements.push(self.statement());
                }
                self.consume(TokenType::RightBrace, "Expected '}' after block.");
                statements
        }

        // ==========================================
        //                EXPRESSIONS
        // ==========================================
        //
        // Binary operators are parsed with a single precedence-climbing
        // loop (`parse_binary`) driven by `infix_precedence`, rather than
        // one hand-written function per precedence level. The original
        // logical_or -> logical_and -> equality -> comparison ->
        // bitwise_or -> bitwise_xor -> bitwise_and -> shift -> additive ->
        // multiplicative chain meant *every* atom -- even a bare `5` --
        // walked through 10 nested function calls before reaching
        // `unary`/`primary`. That's 10 stack frames of pure overhead per
        // token in the common case, and every new operator meant writing
        // a whole new near-identical function. The loop below does the
        // same parse in one function, with recursion only where the
        // grammar actually requires it (parsing the right-hand operand of
        // a tighter-binding operator).

        fn expression(&mut self) -> Expr {
                // Assignments are handled at the statement level, so the
                // lowest-precedence expression form starts at `Or`.
                self.parse_binary(Self::PREC_OR)
        }

        const PREC_OR: u8 = 1;
        const PREC_AND: u8 = 2;
        const PREC_EQUALITY: u8 = 3;
        const PREC_COMPARISON: u8 = 4;
        const PREC_BIT_OR: u8 = 5;
        const PREC_BIT_XOR: u8 = 6;
        const PREC_BIT_AND: u8 = 7;
        const PREC_SHIFT: u8 = 8;
        const PREC_ADDITIVE: u8 = 9;
        const PREC_MULTIPLICATIVE: u8 = 10;

        /// Every binary operator's left-associative precedence, or `None`
        /// if `token_type` isn't an infix operator at all (which is how
        /// the climbing loop knows where to stop).
        fn infix_precedence(token_type: TokenType) -> Option<(u8, BinaryOp)> {
                use TokenType::*;
                Some(match token_type {
                        Or => (Self::PREC_OR, BinaryOp::Or),
                        And => (Self::PREC_AND, BinaryOp::And),
                        EqualEqual => (Self::PREC_EQUALITY, BinaryOp::Eq),
                        BangEqual => (Self::PREC_EQUALITY, BinaryOp::Ne),
                        Less => (Self::PREC_COMPARISON, BinaryOp::Lt),
                        LessEqual => (Self::PREC_COMPARISON, BinaryOp::Le),
                        Greater => (Self::PREC_COMPARISON, BinaryOp::Gt),
                        GreaterEqual => (Self::PREC_COMPARISON, BinaryOp::Ge),
                        Pipe => (Self::PREC_BIT_OR, BinaryOp::BitOr),
                        Xor => (Self::PREC_BIT_XOR, BinaryOp::BitXor),
                        Ampersand => (Self::PREC_BIT_AND, BinaryOp::BitAnd),
                        ShiftLeft => (Self::PREC_SHIFT, BinaryOp::Shl),
                        ShiftRight => (Self::PREC_SHIFT, BinaryOp::Shr),
                        Plus => (Self::PREC_ADDITIVE, BinaryOp::Add),
                        Minus => (Self::PREC_ADDITIVE, BinaryOp::Sub),
                        Star => (Self::PREC_MULTIPLICATIVE, BinaryOp::Mul),
                        Slash => (Self::PREC_MULTIPLICATIVE, BinaryOp::Div),
                        Mod => (Self::PREC_MULTIPLICATIVE, BinaryOp::Mod),
                        _ => return None,
                })
        }

        fn parse_binary(&mut self, min_prec: u8) -> Expr {
                let mut left = self.unary();

                while let Some((prec, op)) = Self::infix_precedence(self.peek().token_type) {
                        if prec < min_prec {
                                break;
                        }
                        self.advance(); // consume the operator
                                         // Left-associative: the right operand only accepts
                                         // strictly tighter-binding operators.
                        let right = Box::new(self.parse_binary(prec + 1));
                        left = Expr::Binary { left: Box::new(left), op, right };
                }

                left
        }

        fn unary(&mut self) -> Expr {
                if self.match_token(&[TokenType::Bang, TokenType::Minus, TokenType::Tilde]) {
                        let op = match self.previous().token_type {
                                TokenType::Minus => UnaryOp::Neg,
                                TokenType::Bang => UnaryOp::Not,
                                TokenType::Tilde => UnaryOp::BitNot,
                                _ => unreachable!(),
                        };
                        let right = Box::new(self.unary());
                        return Expr::Unary { op, expr: right };
                }
                self.exponent()
        }

        fn exponent(&mut self) -> Expr {
                let mut expr = self.postfix();
                if self.match_token(&[TokenType::Pow]) {
                        let right = Box::new(self.exponent()); // right-associative
                        expr = Expr::Binary { left: Box::new(expr), op: BinaryOp::Pow, right };
                }
                expr
        }

        fn postfix(&mut self) -> Expr {
                let mut expr = self.primary();
                while self.match_token(&[TokenType::LeftParen]) {
                        let mut args = Vec::new();
                        if !self.check(TokenType::RightParen) {
                                loop {
                                        args.push(self.expression());
                                        if !self.match_token(&[TokenType::Comma]) {
                                                break;
                                        }
                                }
                        }
                        self.consume(TokenType::RightParen, "Expected ')' after arguments.");
                        expr = Expr::Call { callee: Box::new(expr), args };
                }
                expr
        }

        fn primary(&mut self) -> Expr {
                if self.match_token(&[TokenType::False]) {
                        return Expr::Bool(false);
                }
                if self.match_token(&[TokenType::True]) {
                        return Expr::Bool(true);
                }
                if self.match_token(&[TokenType::Nil]) {
                        return Expr::Nil;
                }

                if self.match_token(&[TokenType::Number]) {
                        // The scanner always attaches the exact raw source
                        // text as `Literal::Str` on Number tokens -- read
                        // that directly rather than resolving `symbol`,
                        // since it's already sitting right there and this
                        // is precisely the string the CAS can't afford to
                        // round anything off of.
                        return Expr::Number(Self::literal_text(self.previous()));
                }
                if self.match_token(&[TokenType::Str]) {
                        return Expr::String(Self::literal_text(self.previous()));
                }
                if self.match_token(&[TokenType::Identifier]) {
                        return Expr::Variable(self.previous().symbol);
                }
                if self.match_token(&[TokenType::LeftParen]) {
                        let expr = self.expression();
                        self.consume(TokenType::RightParen, "Expected ')' after expression.");
                        return Expr::Group(Box::new(expr));
                }

                panic!("Expected expression at {:?}", self.peek());
        }

        fn literal_text(token: &Token) -> String {
                match &token.literal {
                        Some(Literal::Str(s)) => s.clone(),
                        None => panic!(
                                "internal error: {:?} token missing its Literal payload from the scanner",
                                token.token_type
                        ),
                }
        }

        // ==========================================
        //              HELPER METHODS
        // ==========================================

        fn match_token(&mut self, types: &[TokenType]) -> bool {
                for &token_type in types {
                        if self.check(token_type) {
                                self.advance();
                                return true;
                        }
                }
                false
        }

        /// Returns a borrow of the consumed token rather than a clone --
        /// the original cloned `Token` on every call, which meant a fresh
        /// heap allocation (copying the inner `String` out of
        /// `Literal::Str`) for every single token consumed, including
        /// every punctuation and keyword token that never needed it.
        /// Callers here only ever pull a `Copy` field (`symbol`) or read
        /// `literal` immediately, so nothing is lost by borrowing instead.
        fn consume(&mut self, token_type: TokenType, message: &str) -> &Token {
                if self.check(token_type) {
                        return self.advance();
                }
                panic!("Parse error: {}", message); // In a real parser, return a Result/Error
        }

        fn check(&self, token_type: TokenType) -> bool {
                if self.is_at_end() {
                        return false;
                }
                self.peek().token_type == token_type
        }

        fn advance(&mut self) -> &Token {
                if !self.is_at_end() {
                        self.current += 1;
                }
                self.previous()
        }

        fn is_at_end(&self) -> bool {
                self.peek().token_type == TokenType::Eof
        }

        fn peek(&self) -> &Token {
                &self.tokens[self.current]
        }

        fn previous(&self) -> &Token {
                &self.tokens[self.current - 1]
        }

        fn get_assign_op(&self, token_type: TokenType) -> Option<AssignOp> {
                match token_type {
                        TokenType::Equal => Some(AssignOp::Assign),
                        TokenType::PlusEqual => Some(AssignOp::AddAssign),
                        TokenType::MinusEqual => Some(AssignOp::SubAssign),
                        TokenType::StarEqual => Some(AssignOp::MulAssign),
                        TokenType::SlashEqual => Some(AssignOp::DivAssign),
                        TokenType::ModEqual => Some(AssignOp::ModAssign),
                        TokenType::PowEqual => Some(AssignOp::PowAssign),
                        TokenType::AmpersandEqual => Some(AssignOp::BitAndAssign),
                        TokenType::PipeEqual => Some(AssignOp::BitOrAssign),
                        _ => None,
                }
        }
}
