use crate::lexer::token::Token;
use crate::lexer::token_type::TokenType;
use crate::ast::expr::*;
use crate::ast::unary_op::*;
use crate::ast::binary_op::*;
use crate::ast::statement::*;
use crate::ast::assign_op::*;

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
                let name_token = self.consume(TokenType::Identifier, "Expected variable name.");
                let name = name_token.symbol.to_string(); // Assuming symbol represents the text

                let mut value = None;
                if self.match_token(&[TokenType::Equal]) {
                        value = Some(self.expression());
                }

                self.consume(TokenType::Semicolon, "Expected ';' after variable declaration.");
                Statement::Let { name, value }
        }

        fn function_declaration(&mut self) -> Statement {
                //self.consume (TokenType::Fn, "Expected fn before function declaration.");
                let name = self.consume(TokenType::Identifier, "Expected function name.").symbol.to_string();
                self.consume(TokenType::LeftParen, "Expected '(' after function name.");
                
                let mut params = Vec::new();
                if !self.check(TokenType::RightParen) {
                        loop {
                                params.push(self.consume(TokenType::Identifier, "Expected parameter name.").symbol.to_string());
                                if !self.match_token(&[TokenType::Comma]) {
                                        break;
                                }
                        }
                }
                self.consume(TokenType::RightParen, "Expected ')' after parameters.");
                self.consume(TokenType::Equal, "Expected '=' before function body.");
                
                let body = self.expression();
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
                if self.check(TokenType::Identifier) && self.current + 1 < self.tokens.len() {
                        let next_type = self.tokens[self.current + 1].token_type;
                        if let Some(op) = self.get_assign_op(next_type) {
                                let name = self.advance().symbol.to_string(); // Consume Identifier
                                self.advance(); // Consume Op
                                let value = self.expression();
                                
                                // Assuming your AST uses Statement::Assign { name, op, value }
                                return Statement::Assign { name, op, value }; 
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
        fn expression(&mut self) -> Expr {
                // Assignments are now handled cleanly at the statement level, 
                // so the base expression is just the lowest precedence operator!
                self.logical_or()
        }
        
        fn logical_or(&mut self) -> Expr {
                let mut expr = self.logical_and();
                while self.match_token(&[TokenType::Or]) {
                        let right = Box::new(self.logical_and());
                        expr = Expr::Binary { left: Box::new(expr), op: BinaryOp::Or, right };
                }
                expr
        }

        fn logical_and(&mut self) -> Expr {
                let mut expr = self.equality();
                while self.match_token(&[TokenType::And]) {
                        let right = Box::new(self.equality());
                        expr = Expr::Binary { left: Box::new(expr), op: BinaryOp::And, right };
                }
                expr
        }

        fn equality(&mut self) -> Expr {
                let mut expr = self.comparison();
                while self.match_token(&[TokenType::EqualEqual, TokenType::BangEqual]) {
                        let operator = self.get_binary_op(self.previous().token_type);
                        let right = Box::new(self.comparison());
                        expr = Expr::Binary { left: Box::new(expr), op: operator, right };
                }
                expr
        }

        fn comparison(&mut self) -> Expr {
                let mut expr = self.bitwise_or();
                while self.match_token(&[TokenType::Greater, TokenType::GreaterEqual, TokenType::Less, TokenType::LessEqual]) {
                        let operator = self.get_binary_op(self.previous().token_type);
                        let right = Box::new(self.bitwise_or());
                        expr = Expr::Binary { left: Box::new(expr), op: operator, right };
                }
                expr
        }

        fn bitwise_or(&mut self) -> Expr {
                let mut expr = self.bitwise_xor();
                while self.match_token(&[TokenType::Pipe]) {
                        let right = Box::new(self.bitwise_xor());
                        expr = Expr::Binary { left: Box::new(expr), op: BinaryOp::BitOr, right };
                }
                expr
        }

        fn bitwise_xor(&mut self) -> Expr {
                let mut expr = self.bitwise_and();
                while self.match_token(&[TokenType::Xor]) {
                        let right = Box::new(self.bitwise_and());
                        expr = Expr::Binary { left: Box::new(expr), op: BinaryOp::BitXor, right };
                }
                expr
        }

        fn bitwise_and(&mut self) -> Expr {
                let mut expr = self.shift();
                while self.match_token(&[TokenType::Ampersand]) {
                        let right = Box::new(self.shift());
                        expr = Expr::Binary { left: Box::new(expr), op: BinaryOp::BitAnd, right };
                }
                expr
        }

        fn shift(&mut self) -> Expr {
                let mut expr = self.additive();
                while self.match_token(&[TokenType::ShiftLeft, TokenType::ShiftRight]) {
                        let operator = self.get_binary_op(self.previous().token_type);
                        let right = Box::new(self.additive());
                        expr = Expr::Binary { left: Box::new(expr), op: operator, right };
                }
                expr
        }

        fn additive(&mut self) -> Expr {
                let mut expr = self.multiplicative();
                while self.match_token(&[TokenType::Plus, TokenType::Minus]) {
                        let operator = self.get_binary_op(self.previous().token_type);
                        let right = Box::new(self.multiplicative());
                        expr = Expr::Binary { left: Box::new(expr), op: operator, right };
                }
                expr
        }

        fn multiplicative(&mut self) -> Expr {
                let mut expr = self.unary();
                while self.match_token(&[TokenType::Star, TokenType::Slash, TokenType::Mod]) {
                        let operator = self.get_binary_op(self.previous().token_type);
                        let right = Box::new(self.unary());
                        expr = Expr::Binary { left: Box::new(expr), op: operator, right };
                }
                expr
        }

        fn unary(&mut self) -> Expr {
                if self.match_token(&[TokenType::Bang, TokenType::Minus, TokenType::Tilde]) {
                        let op = match self.previous().token_type {
                                TokenType::Minus => UnaryOp::Neg,
                                TokenType::Bang  => UnaryOp::Not,
                                TokenType::Tilde => UnaryOp::BitNot, // Or Tilde depending on mapping
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
                        let right = Box::new(self.exponent());
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
                if self.match_token(&[TokenType::False]) { return Expr::Bool(false); }
                if self.match_token(&[TokenType::True]) { return Expr::Bool(true); }
                if self.match_token(&[TokenType::Nil]) { return Expr::Nil; }

                if self.match_token(&[TokenType::Number]) {
                        return Expr::Number(self.previous().symbol.to_string());
                }
                if self.match_token(&[TokenType::Str]) {
                        return Expr::String(self.previous().symbol.to_string());
                }
                if self.match_token(&[TokenType::Identifier]) {
                        return Expr::Variable(self.previous().symbol.to_string());
                }
                if self.match_token(&[TokenType::LeftParen]) {
                        let expr = self.expression();
                        self.consume(TokenType::RightParen, "Expected ')' after expression.");
                        return Expr::Group(Box::new(expr));
                }

                panic!("Expected expression at {:?}", self.peek());
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

        fn consume(&mut self, token_type: TokenType, message: &str) -> Token {
                if self.check(token_type) {
                        return self.advance().clone();
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

        fn get_binary_op(&self, token_type: TokenType) -> BinaryOp {
                match token_type {
                        TokenType::Plus => BinaryOp::Add,
                        TokenType::Minus => BinaryOp::Sub,
                        TokenType::Star => BinaryOp::Mul,
                        TokenType::Slash => BinaryOp::Div,
                        TokenType::Mod => BinaryOp::Mod,
                        TokenType::Pow => BinaryOp::Pow,
                        TokenType::EqualEqual => BinaryOp::Eq,
                        TokenType::BangEqual => BinaryOp::Ne,
                        TokenType::Less => BinaryOp::Lt,
                        TokenType::LessEqual => BinaryOp::Le,
                        TokenType::Greater => BinaryOp::Gt,
                        TokenType::GreaterEqual => BinaryOp::Ge,
                        TokenType::ShiftLeft => BinaryOp::Shl,
                        TokenType::ShiftRight => BinaryOp::Shr,
                        _ => unreachable!(),
                }
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
