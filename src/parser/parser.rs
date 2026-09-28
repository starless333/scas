use crate::ast::assign_op::*;
use crate::ast::binary_op::*;
use crate::ast::expr::*;
use crate::ast::statement::*;
use crate::ast::unary_op::*;
use crate::lexer::literal::Literal;
use crate::lexer::token::Token;
use crate::lexer::token_type::TokenType;
use crate::parser::error::ParseError;

type ParseResult<T> = Result<T, ParseError>;

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
    pub fn parse(&mut self) -> ParseResult<Vec<Statement>> {
        let mut statements = Vec::new();
        while !self.is_at_end() {
            statements.push(self.statement()?);
        }
        Ok(statements)
    }

    // ==========================================
    //                STATEMENTS
    // ==========================================

    fn statement(&mut self) -> ParseResult<Statement> {
        let span = self.peek_span();
        let kind = self.statement_kind()?;
        Ok(Statement::new(span, kind))
    }

    fn statement_kind(&mut self) -> ParseResult<StatementKind> {
        if self.match_token(&[TokenType::Let]) {
            return self.variable_declaration();
        }
        if self.match_token(&[TokenType::Fn]) {
            return self.function_declaration();
        }
        if self.looks_like_math_function_declaration() {
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
            self.consume(TokenType::Semicolon, "expected `;` after `break`")?;
            return Ok(StatementKind::Break);
        }
        if self.match_token(&[TokenType::Continue]) {
            self.consume(TokenType::Semicolon, "expected `;` after `continue`")?;
            return Ok(StatementKind::Continue);
        }
        if self.match_token(&[TokenType::Print]) {
            return self.print_statement();
        }
        if self.match_token(&[TokenType::LeftBrace]) {
            return Ok(StatementKind::Block(self.block()?));
        }

        self.expression_statement()
    }

    fn variable_declaration(&mut self) -> ParseResult<StatementKind> {
        // `.symbol` is a `Copy` u32 id from the interner -- no
        // allocation, no need to touch the symbol table here.
        let name = self
            .consume(TokenType::Identifier, "expected a variable name")?
            .symbol;

        let mut value = None;
        if self.match_token(&[TokenType::Equal]) {
            value = Some(self.expression()?);
        }

        self.consume(
            TokenType::Semicolon,
            "expected `;` after variable declaration",
        )?;
        Ok(StatementKind::Let { name, value })
    }

    fn function_declaration(&mut self) -> ParseResult<StatementKind> {
        let name = self
            .consume(TokenType::Identifier, "expected a function name")?
            .symbol;
        self.consume(TokenType::LeftParen, "expected `(` after function name")?;

        let mut params = Vec::new();
        if !self.check(TokenType::RightParen) {
            loop {
                params.push(
                    self.consume(TokenType::Identifier, "expected a parameter name")?
                        .symbol,
                );
                if !self.match_token(&[TokenType::Comma]) {
                    break;
                }
            }
        }
        self.consume(TokenType::RightParen, "expected `)` after parameters")?;
        let body = if self.match_token(&[TokenType::Equal]) {
            let expression = self.expression()?;
            self.consume(TokenType::Semicolon, "expected `;` after function body")?;
            FunctionBody::Expression(expression)
        } else if self.match_token(&[TokenType::LeftBrace]) {
            FunctionBody::Block(self.block()?)
        } else {
            return Err(self.error("expected `=` or `{` before function body"));
        };
        Ok(StatementKind::Function { name, params, body })
    }

    /// Recognizes the CAS-style shorthand `f(x, y) = expression` without
    /// confusing an ordinary call such as `f(x, y);` for a declaration.
    fn looks_like_math_function_declaration(&self) -> bool {
        if !self.check(TokenType::Identifier)
            || self.tokens.get(self.current + 1).map(|t| t.token_type) != Some(TokenType::LeftParen)
        {
            return false;
        }
        let mut index = self.current + 2;
        let mut expect_parameter = true;
        loop {
            let Some(token) = self.tokens.get(index) else {
                return false;
            };
            match token.token_type {
                TokenType::RightParen if expect_parameter || index > self.current + 2 => {
                    return self.tokens.get(index + 1).map(|t| t.token_type)
                        == Some(TokenType::Equal);
                }
                TokenType::Identifier if expect_parameter => expect_parameter = false,
                TokenType::Comma if !expect_parameter => expect_parameter = true,
                _ => return false,
            }
            index += 1;
        }
    }

    fn if_statement(&mut self) -> ParseResult<StatementKind> {
        let condition = self.expression()?;

        self.consume(TokenType::LeftBrace, "expected `{` before `if` body")?;
        let then_branch = Box::new(Statement::new(
            self.previous_span(),
            StatementKind::Block(self.block()?),
        ));

        let mut else_branch = None;
        if self.match_token(&[TokenType::Else]) {
            self.consume(TokenType::LeftBrace, "expected `{` before `else` body")?;
            else_branch = Some(Box::new(Statement::new(
                self.previous_span(),
                StatementKind::Block(self.block()?),
            )));
        }

        Ok(StatementKind::If {
            condition,
            then_branch,
            else_branch,
        })
    }

    fn while_statement(&mut self) -> ParseResult<StatementKind> {
        let condition = self.expression()?;
        self.consume(TokenType::LeftBrace, "expected `{` before `while` body")?;
        let body = Box::new(Statement::new(
            self.previous_span(),
            StatementKind::Block(self.block()?),
        ));

        Ok(StatementKind::While { condition, body })
    }

    fn for_statement(&mut self) -> ParseResult<StatementKind> {
        let init_span = self.peek_span();
        let init = Box::new(Statement::new(
            init_span,
            if self.match_token(&[TokenType::Let]) {
                self.variable_declaration()? // This consumes its own ';'
            } else {
                self.expression_statement()? // This consumes its own ';'
            },
        ));

        let condition = self.expression()?;
        self.consume(
            TokenType::Semicolon,
            "expected `;` after for loop condition",
        )?;

        // Parse the update (e.g., i += 1) WITHOUT expecting a semicolon afterwards
        let update_span = self.peek_span();
        let update = Box::new(Statement::new(
            update_span,
            self.parse_assignment_or_expr()?,
        ));

        self.consume(TokenType::LeftBrace, "expected `{` before `for` body")?;
        let body = Box::new(Statement::new(
            self.previous_span(),
            StatementKind::Block(self.block()?),
        ));

        Ok(StatementKind::For {
            init,
            condition,
            update,
            body,
        })
    }

    fn return_statement(&mut self) -> ParseResult<StatementKind> {
        let value = if !self.check(TokenType::Semicolon) {
            Some(self.expression()?)
        } else {
            None
        };
        self.consume(TokenType::Semicolon, "expected `;` after return value")?;
        Ok(StatementKind::Return(value))
    }

    fn print_statement(&mut self) -> ParseResult<StatementKind> {
        self.consume(TokenType::LeftParen, "expected `(` after `print`")?;
        let value = self.expression()?;
        self.consume(TokenType::RightParen, "expected `)` after print expression")?;
        self.consume(TokenType::Semicolon, "expected `;` after value")?;
        Ok(StatementKind::Print(value))
    }

    fn expression_statement(&mut self) -> ParseResult<StatementKind> {
        let stmt = self.parse_assignment_or_expr()?;
        self.consume(TokenType::Semicolon, "expected `;` after statement")?;
        Ok(stmt)
    }

    /// Helper to parse either an assignment OR an expression, without consuming a trailing semicolon.
    /// This is crucial because `for` loop updates don't have semicolons, but normal statements do.
    fn parse_assignment_or_expr(&mut self) -> ParseResult<StatementKind> {
        // Lookahead to check if this is an assignment statement (Identifier Operator Expr)
        if self.check(TokenType::Identifier) {
            if let Some(next) = self.tokens.get(self.current + 1) {
                if let Some(op) = self.get_assign_op(next.token_type) {
                    let name = self.advance().symbol; // Consume Identifier
                    self.advance(); // Consume Op
                    let value = self.expression()?;
                    return Ok(StatementKind::Assign { name, op, value });
                }
            }
        }

        // If it's not an assignment, it's just a normal expression
        Ok(StatementKind::Expr(self.expression()?))
    }

    fn block(&mut self) -> ParseResult<Vec<Statement>> {
        let mut statements = Vec::new();
        while !self.check(TokenType::RightBrace) && !self.is_at_end() {
            statements.push(self.statement()?);
        }
        self.consume(TokenType::RightBrace, "expected `}` after block")?;
        Ok(statements)
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

    fn expression(&mut self) -> ParseResult<Expr> {
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

    fn parse_binary(&mut self, min_prec: u8) -> ParseResult<Expr> {
        let mut left = self.unary()?;

        loop {
            let (prec, op, explicit) =
                if let Some((prec, op)) = Self::infix_precedence(self.peek().token_type) {
                    (prec, op, true)
                } else if matches!(
                    self.peek().token_type,
                    TokenType::Number | TokenType::Identifier | TokenType::LeftParen
                ) {
                    // Mathematical adjacency: `2x`, `3(x + 1)`, and
                    // `(x + 1)y` are multiplication.
                    (Self::PREC_MULTIPLICATIVE, BinaryOp::Mul, false)
                } else {
                    break;
                };
            if prec < min_prec {
                break;
            }
            if explicit {
                self.advance(); // consume the operator
            }
            // Left-associative: the right operand only accepts
            // strictly tighter-binding operators.
            let right = Box::new(self.parse_binary(prec + 1)?);
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right,
            };
        }

        Ok(left)
    }

    fn unary(&mut self) -> ParseResult<Expr> {
        if self.match_token(&[TokenType::Bang, TokenType::Minus, TokenType::Tilde]) {
            let op = match self.previous().token_type {
                TokenType::Minus => UnaryOp::Neg,
                TokenType::Bang => UnaryOp::Not,
                TokenType::Tilde => UnaryOp::BitNot,
                _ => return Err(self.error("internal parser error: invalid unary operator")),
            };
            let right = Box::new(self.unary()?);
            return Ok(Expr::Unary { op, expr: right });
        }
        self.exponent()
    }

    fn exponent(&mut self) -> ParseResult<Expr> {
        let mut expr = self.postfix()?;
        if self.match_token(&[TokenType::Pow]) {
            // Parsing the RHS through unary keeps exponentiation
            // right-associative while accepting `2 ^ -3`.
            let right = Box::new(self.unary()?);
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::Pow,
                right,
            };
        }
        Ok(expr)
    }

    fn postfix(&mut self) -> ParseResult<Expr> {
        let mut expr = self.primary()?;
        loop {
            let callable = matches!(
                expr,
                Expr::Variable(_) | Expr::Call { .. } | Expr::Index { .. }
            );
            if callable && self.match_token(&[TokenType::LeftParen]) {
                let mut args = Vec::new();
                if !self.check(TokenType::RightParen) {
                    loop {
                        args.push(self.expression()?);
                        if !self.match_token(&[TokenType::Comma]) {
                            break;
                        }
                    }
                }
                self.consume(TokenType::RightParen, "expected `)` after arguments")?;
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                };
            } else if self.match_token(&[TokenType::LeftBracket]) {
                let index = self.expression()?;
                self.consume(TokenType::RightBracket, "expected `]` after array index")?;
                expr = Expr::Index {
                    target: Box::new(expr),
                    index: Box::new(index),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn primary(&mut self) -> ParseResult<Expr> {
        if self.match_token(&[TokenType::False]) {
            return Ok(Expr::Bool(false));
        }
        if self.match_token(&[TokenType::True]) {
            return Ok(Expr::Bool(true));
        }
        if self.match_token(&[TokenType::Nil]) {
            return Ok(Expr::Nil);
        }

        if self.match_token(&[TokenType::Number]) {
            // The scanner always attaches the exact raw source
            // text as `Literal::Str` on Number tokens -- read
            // that directly rather than resolving `symbol`,
            // since it's already sitting right there and this
            // is precisely the string the CAS can't afford to
            // round anything off of.
            return Ok(Expr::Number(Self::literal_text(self.previous())?));
        }
        if self.match_token(&[TokenType::Str]) {
            return Ok(Expr::String(Self::literal_text(self.previous())?));
        }
        if self.match_token(&[TokenType::Identifier]) {
            return Ok(Expr::Variable(self.previous().symbol));
        }
        if self.match_token(&[TokenType::LeftBracket]) {
            let mut elements = Vec::new();
            if !self.check(TokenType::RightBracket) {
                loop {
                    elements.push(self.expression()?);
                    if !self.match_token(&[TokenType::Comma]) {
                        break;
                    }
                    if self.check(TokenType::RightBracket) {
                        break;
                    }
                }
            }
            self.consume(TokenType::RightBracket, "expected `]` after array literal")?;
            return Ok(Expr::Array(elements));
        }
        if self.match_token(&[TokenType::LeftParen]) {
            let expr = self.expression()?;
            self.consume(TokenType::RightParen, "expected `)` after expression")?;
            return Ok(Expr::Group(Box::new(expr)));
        }

        Err(self.error("expected an expression"))
    }

    fn literal_text(token: &Token) -> ParseResult<String> {
        match &token.literal {
            Some(Literal::Str(s)) => Ok(s.clone()),
            None => Err(ParseError::at(
                token,
                format!(
                    "internal parser error: {:?} token has no literal",
                    token.token_type
                ),
            )),
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
    fn consume(&mut self, token_type: TokenType, message: &str) -> ParseResult<&Token> {
        if self.check(token_type) {
            return Ok(self.advance());
        }
        Err(ParseError::at(self.peek(), message))
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError::at(self.peek(), message)
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

    fn peek_span(&self) -> SourceSpan {
        let token = self.peek();
        SourceSpan {
            line: token.line,
            column: token.column,
            length: token.length.max(1),
        }
    }

    fn previous_span(&self) -> SourceSpan {
        let token = self.previous();
        SourceSpan {
            line: token.line,
            column: token.column,
            length: token.length.max(1),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::scanner::Scanner;

    fn parse_error(source: &str) -> ParseError {
        let mut scanner = Scanner::new(source);
        let tokens = scanner.scan_tokens().clone();
        assert!(!scanner.had_error(), "test input must lex successfully");
        Parser::new(tokens)
            .parse()
            .expect_err("source should not parse")
    }

    #[test]
    fn reports_the_unexpected_token_with_a_source_span() {
        let source = "let value = 1;\nprint(value + );";
        let error = parse_error(source);

        assert_eq!(error.message, "expected an expression");
        assert_eq!((error.line, error.column, error.length), (2, 15, 1));
        assert_eq!(error.found, TokenType::RightParen);

        let rendered = error.render(source, "example.scas");
        assert!(rendered.contains("error: expected an expression"));
        assert!(rendered.contains("--> example.scas:2:15"));
        assert!(rendered.contains("2 | print(value + );"));
        assert!(rendered.contains("^ unexpected `)`"));
    }

    #[test]
    fn reports_end_of_input_without_panicking() {
        let source = "if true {\n    print(1);";
        let error = parse_error(source);

        assert_eq!(error.message, "expected `}` after block");
        assert_eq!(error.found, TokenType::Eof);
        assert_eq!((error.line, error.column, error.length), (2, 14, 0));
        assert!(error
            .render(source, "<stdin>")
            .contains("unexpected end of input"));
    }

    #[test]
    fn reports_a_missing_declaration_name_at_the_right_column() {
        let error = parse_error("let = 3;");
        assert_eq!(error.message, "expected a variable name");
        assert_eq!((error.line, error.column), (1, 5));
        assert_eq!(error.found, TokenType::Equal);
    }

    #[test]
    fn for_headers_require_semicolon_separators() {
        let mut scanner = Scanner::new("for let i = 0; i < 3; i += 1 { print(i); }");
        let tokens = scanner.scan_tokens().clone();
        let program = Parser::new(tokens)
            .parse()
            .expect("semicolon form should parse");
        assert!(matches!(program[0].kind, StatementKind::For { .. }));

        let error = parse_error("for let i = 0, i < 3, i += 1 { }");
        assert!(error.message.contains("expected `;`"));
    }
}
