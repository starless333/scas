use crate::ast::assign_op::*;
use crate::ast::binary_op::*;
use crate::ast::expr::*;
use crate::ast::span::SourceSpan;
use crate::ast::statement::*;
use crate::ast::unary_op::*;
use crate::lexer::literal::Literal;
use crate::lexer::token::Token;
use crate::lexer::token_type::TokenType;
use crate::parser::error::ParseError;

type ParseResult<T> = Result<T, ParseError>;

#[derive(Debug, Clone, PartialEq)]
pub struct ParseReport {
    pub statements: Vec<Statement>,
    pub errors: Vec<ParseError>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
    /// A one-parameter expression-bodied function supplies that parameter as
    /// the square size for `(entry)_(i,j)` matrix rules.
    matrix_rule_dimension: Option<crate::lexer::symbol::Symbol>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            current: 0,
            matrix_rule_dimension: None,
        }
    }

    /// Parses the entire program into a sequence of statements.
    pub fn parse(&mut self) -> ParseResult<Vec<Statement>> {
        let mut statements = Vec::new();
        while !self.is_at_end() {
            statements.push(self.statement()?);
        }
        Ok(statements)
    }

    /// Parse as much input as possible, retaining successfully parsed
    /// statements and collecting diagnostics after synchronization points.
    /// Editors use this for half-typed buffers; batch execution keeps the
    /// fail-fast `parse` behavior.
    pub fn parse_recovering(&mut self) -> ParseReport {
        let mut statements = Vec::new();
        let mut errors = Vec::new();
        while !self.is_at_end() {
            match self.statement() {
                Ok(statement) => statements.push(statement),
                Err(error) => {
                    errors.push(error);
                    self.synchronize();
                }
            }
        }
        ParseReport { statements, errors }
    }

    fn synchronize(&mut self) {
        while !self.is_at_end() {
            if self.current > 0 && self.previous().token_type == TokenType::Semicolon {
                return;
            }
            if matches!(
                self.peek().token_type,
                TokenType::Let
                    | TokenType::Fn
                    | TokenType::If
                    | TokenType::While
                    | TokenType::For
                    | TokenType::Print
                    | TokenType::Return
                    | TokenType::DocComment
                    | TokenType::RenderLeftBrace
                    | TokenType::RenderLeftBracket
            ) {
                return;
            }
            self.advance();
        }
    }

    // ==========================================
    //                STATEMENTS
    // ==========================================

    fn statement(&mut self) -> ParseResult<Statement> {
        let start = self.peek_span();
        let kind = self.statement_kind()?;
        Ok(Statement::new(start.through(self.previous_span()), kind))
    }

    fn statement_kind(&mut self) -> ParseResult<StatementKind> {
        if self.match_token(&[TokenType::Import]) {
            let path = self.consume(
                TokenType::Str,
                "expected a quoted module path after `import`",
            )?;
            let path = Self::literal_text(path)?;
            self.consume(TokenType::Semicolon, "expected `;` after import path")?;
            return Ok(StatementKind::Import(path));
        }
        if self.match_token(&[TokenType::DocComment]) {
            let mut prose = Self::literal_text(self.previous())?;
            while !prose.is_empty()
                && self.check(TokenType::DocComment)
                && self.peek().line == self.previous().line + 1
                && !Self::literal_text(self.peek())?.is_empty()
            {
                self.advance();
                prose.push('\n');
                prose.push_str(&Self::literal_text(self.previous())?);
            }
            return Ok(StatementKind::DocComment(prose));
        }
        if self.match_token(&[TokenType::RenderLeftBrace]) {
            let mut statements = Vec::new();
            while !self.check(TokenType::RenderRightBrace) && !self.is_at_end() {
                statements.push(self.statement()?);
            }
            self.consume(
                TokenType::RenderRightBrace,
                "expected `\\}` after render block",
            )?;
            return Ok(StatementKind::RenderBlock(statements));
        }
        if self.match_token(&[TokenType::RenderLeftBracket]) {
            let expression = self.expression()?;
            let show_result = self.match_token(&[TokenType::FatArrow]);
            self.consume(
                TokenType::RenderRightBracket,
                "expected `\\]` after render line",
            )?;
            return Ok(StatementKind::RenderLine {
                expression,
                show_result,
            });
        }
        if self.match_token(&[TokenType::Let]) {
            if self.check(TokenType::Identifier)
                && self
                    .tokens
                    .get(self.current + 1)
                    .map(|token| token.token_type)
                    == Some(TokenType::LeftParen)
            {
                return self.function_declaration();
            }
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
        let name_token = self
            .consume(TokenType::Identifier, "expected a variable name")?
            .clone();
        let name = name_token.symbol;

        let mut value = None;
        if self.match_token(&[TokenType::Equal]) {
            value = Some(self.expression()?);
        }

        let first = Statement::new(
            SourceSpan::from_token(&name_token).through(self.previous_span()),
            StatementKind::Let { name, value },
        );
        if !self.match_token(&[TokenType::Comma]) {
            self.consume(
                TokenType::Semicolon,
                "expected `;` after variable declaration",
            )?;
            return Ok(first.kind);
        }

        let mut declarations = vec![first];
        loop {
            // `let a = 1, b = 2;` deliberately mirrors ordinary mathematical
            // declaration lists; repeating `let` would make the comma noisy.
            let start = self.peek_span();
            let name = self
                .consume(TokenType::Identifier, "expected a variable name after `,`")?
                .symbol;
            let value = if self.match_token(&[TokenType::Equal]) {
                Some(self.expression()?)
            } else {
                None
            };
            declarations.push(Statement::new(
                start.through(self.previous_span()),
                StatementKind::Let { name, value },
            ));
            if !self.match_token(&[TokenType::Comma]) {
                break;
            }
        }
        self.consume(TokenType::Semicolon, "expected `;` after declaration list")?;
        Ok(StatementKind::Chain(declarations))
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
            let previous_dimension = self.matrix_rule_dimension;
            self.matrix_rule_dimension = if params.len() == 1 {
                Some(params[0])
            } else {
                None
            };
            let expression = self.expression()?;
            self.matrix_rule_dimension = previous_dimension;
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
        let branch_start = self.previous_span();
        let branch_kind = StatementKind::Block(self.block()?);
        let then_branch = Box::new(Statement::new(
            branch_start.through(self.previous_span()),
            branch_kind,
        ));

        let mut else_branch = None;
        if self.match_token(&[TokenType::Else]) {
            let span = self.peek_span();
            if self.match_token(&[TokenType::If]) {
                let kind = self.if_statement()?;
                else_branch = Some(Box::new(Statement::new(
                    span.through(self.previous_span()),
                    kind,
                )));
            } else {
                self.consume(TokenType::LeftBrace, "expected `if` or `{` after `else`")?;
                let branch_start = self.previous_span();
                let branch_kind = StatementKind::Block(self.block()?);
                else_branch = Some(Box::new(Statement::new(
                    branch_start.through(self.previous_span()),
                    branch_kind,
                )));
            }
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
        let body_start = self.previous_span();
        let body_kind = StatementKind::Block(self.block()?);
        let body = Box::new(Statement::new(
            body_start.through(self.previous_span()),
            body_kind,
        ));

        Ok(StatementKind::While { condition, body })
    }

    fn for_statement(&mut self) -> ParseResult<StatementKind> {
        let init_span = self.peek_span();
        let init_kind = if self.match_token(&[TokenType::Let]) {
            self.variable_declaration()? // This consumes its own ';'
        } else {
            self.expression_statement()? // This consumes its own ';'
        };
        let init = Box::new(Statement::new(
            init_span.through(self.previous_span()),
            init_kind,
        ));

        let condition = self.expression()?;
        self.consume(
            TokenType::Semicolon,
            "expected `;` after for loop condition",
        )?;

        // Parse the update (e.g., i += 1) WITHOUT expecting a semicolon afterwards
        let update_span = self.peek_span();
        let update_kind = self.parse_assignment_or_expr()?;
        let update = Box::new(Statement::new(
            update_span.through(self.previous_span()),
            update_kind,
        ));

        self.consume(TokenType::LeftBrace, "expected `{` before `for` body")?;
        let body_start = self.previous_span();
        let body_kind = StatementKind::Block(self.block()?);
        let body = Box::new(Statement::new(
            body_start.through(self.previous_span()),
            body_kind,
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
        let first_start = self.peek_span();
        let stmt = self.parse_assignment_or_expr()?;
        if self.match_token(&[TokenType::FatArrow]) {
            let StatementKind::Expr(expression) = stmt else {
                return Err(self.error("`=>` can only follow an expression"));
            };
            self.consume(TokenType::Semicolon, "expected `;` after `=>`")?;
            return Ok(StatementKind::Show(expression));
        }
        if self.match_token(&[TokenType::Comma]) {
            let mut statements = vec![Statement::new(
                first_start.through(self.previous_span()),
                stmt,
            )];
            loop {
                let start = self.peek_span();
                let kind = self.parse_assignment_or_expr()?;
                statements.push(Statement::new(start.through(self.previous_span()), kind));
                if !self.match_token(&[TokenType::Comma]) {
                    break;
                }
            }
            self.consume(TokenType::Semicolon, "expected `;` after command chain")?;
            return Ok(StatementKind::Chain(statements));
        }
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
            let span = left.span.through(right.span);
            left = Expr::new(
                span,
                ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right,
                    implicit: !explicit,
                },
            );
        }

        Ok(left)
    }

    fn unary(&mut self) -> ParseResult<Expr> {
        if self.match_token(&[TokenType::Bang, TokenType::Minus, TokenType::Tilde]) {
            let start = self.previous_span();
            let op = match self.previous().token_type {
                TokenType::Minus => UnaryOp::Neg,
                TokenType::Bang => UnaryOp::Not,
                TokenType::Tilde => UnaryOp::BitNot,
                _ => return Err(self.error("internal parser error: invalid unary operator")),
            };
            let right = Box::new(self.unary()?);
            return Ok(Expr::new(
                start.through(right.span),
                ExprKind::Unary { op, expr: right },
            ));
        }
        self.exponent()
    }

    fn exponent(&mut self) -> ParseResult<Expr> {
        let mut expr = self.postfix()?;
        if self.match_token(&[TokenType::Pow]) {
            // Parsing the RHS through unary keeps exponentiation
            // right-associative while accepting `2 ^ -3`.
            let right = Box::new(self.unary()?);
            let span = expr.span.through(right.span);
            expr = Expr::new(
                span,
                ExprKind::Binary {
                    left: Box::new(expr),
                    op: BinaryOp::Pow,
                    right,
                    implicit: false,
                },
            );
        }
        Ok(expr)
    }

    fn postfix(&mut self) -> ParseResult<Expr> {
        let mut expr = self.primary()?;
        loop {
            let callable = matches!(
                expr.kind,
                ExprKind::Variable(_) | ExprKind::Call { .. } | ExprKind::Index { .. }
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
                let end = SourceSpan::from_token(
                    self.consume(TokenType::RightParen, "expected `)` after arguments")?,
                );
                let span = expr.span.through(end);
                expr = Expr::new(
                    span,
                    ExprKind::Call {
                        callee: Box::new(expr),
                        args,
                    },
                );
            } else if self.match_token(&[TokenType::LeftBracket]) {
                let mut indices = vec![self.expression()?];
                while self.match_token(&[TokenType::Comma]) {
                    indices.push(self.expression()?);
                }
                let end = SourceSpan::from_token(
                    self.consume(TokenType::RightBracket, "expected `]` after array index")?,
                );
                for index in indices {
                    let span = expr.span.through(end);
                    expr = Expr::new(
                        span,
                        ExprKind::Index {
                            target: Box::new(expr),
                            index: Box::new(index),
                        },
                    );
                }
            } else if self.match_token(&[TokenType::Apostrophe]) {
                let span = expr.span.through(self.previous_span());
                expr = Expr::new(
                    span,
                    ExprKind::Unary {
                        op: UnaryOp::Transpose,
                        expr: Box::new(expr),
                    },
                );
            } else if self.match_token(&[TokenType::Subscript]) {
                let subscript = self.previous().clone();
                self.consume(
                    TokenType::LeftParen,
                    "expected `(` after matrix-rule subscript `_`",
                )?;
                let row = self.expression()?;
                self.consume(TokenType::Comma, "expected `,` after the row index")?;
                let column = self.expression()?;
                let mut args = vec![expr, row, column];
                while self.match_token(&[TokenType::Comma]) {
                    args.push(self.expression()?);
                }
                let end = SourceSpan::from_token(
                    self.consume(TokenType::RightParen, "expected `)` after matrix rule")?,
                );
                if args.len() == 3 {
                    if let Some(dimension) = self.matrix_rule_dimension {
                        args.push(Expr::new(end, ExprKind::Variable(dimension)));
                    }
                }
                if !(4..=5).contains(&args.len()) {
                    return Err(self.error(
                        "matrix rule expects (row, column, size) or (row, column, rows, columns)",
                    ));
                }
                let span = args[0].span.through(end);
                expr = Expr::new(
                    span,
                    ExprKind::Call {
                        callee: Box::new(Expr::new(
                            SourceSpan::from_token(&subscript),
                            ExprKind::Variable(subscript.symbol),
                        )),
                        args,
                    },
                );
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn primary(&mut self) -> ParseResult<Expr> {
        if self.match_token(&[TokenType::False]) {
            return Ok(Expr::new(self.previous_span(), ExprKind::Bool(false)));
        }
        if self.match_token(&[TokenType::True]) {
            return Ok(Expr::new(self.previous_span(), ExprKind::Bool(true)));
        }
        if self.match_token(&[TokenType::Nil]) {
            return Ok(Expr::new(self.previous_span(), ExprKind::Nil));
        }

        if self.match_token(&[TokenType::Number]) {
            // The scanner always attaches the exact raw source
            // text as `Literal::Str` on Number tokens -- read
            // that directly rather than resolving `symbol`,
            // since it's already sitting right there and this
            // is precisely the string the CAS can't afford to
            // round anything off of.
            return Ok(Expr::new(
                self.previous_span(),
                ExprKind::Number(Self::literal_text(self.previous())?),
            ));
        }
        if self.match_token(&[TokenType::Str]) {
            return Ok(Expr::new(
                self.previous_span(),
                ExprKind::String(Self::literal_text(self.previous())?),
            ));
        }
        if self.match_token(&[TokenType::Identifier]) {
            return Ok(Expr::new(
                self.previous_span(),
                ExprKind::Variable(self.previous().symbol),
            ));
        }
        if self.match_token(&[TokenType::LeftBracket]) {
            let start = self.previous_span();
            let mut rows = vec![Vec::new()];
            if !self.check(TokenType::RightBracket) {
                loop {
                    rows.last_mut()
                        .expect("matrix always has a row")
                        .push(self.expression()?);
                    if self.match_token(&[TokenType::Semicolon]) {
                        rows.push(Vec::new());
                    } else if !self.match_token(&[TokenType::Comma]) {
                        break;
                    }
                    if self.check(TokenType::RightBracket) {
                        break;
                    }
                }
            }
            let end = SourceSpan::from_token(
                self.consume(TokenType::RightBracket, "expected `]` after array literal")?,
            );
            let span = start.through(end);
            if rows.len() == 1 {
                return Ok(Expr::new(
                    span,
                    ExprKind::Array(rows.pop().expect("single array row")),
                ));
            }
            let columns = rows[0].len();
            if columns == 0 || rows.iter().any(|row| row.len() != columns) {
                return Err(self.error("matrix rows must have the same non-zero length"));
            }
            let rows = rows
                .into_iter()
                .map(|elements| {
                    let row_span = elements
                        .first()
                        .expect("matrix rows are non-empty")
                        .span
                        .through(elements.last().expect("matrix rows are non-empty").span);
                    Expr::new(row_span, ExprKind::Array(elements))
                })
                .collect();
            return Ok(Expr::new(span, ExprKind::Array(rows)));
        }
        if self.match_token(&[TokenType::LeftParen]) {
            let start = self.previous_span();
            let expr = self.expression()?;
            let end = SourceSpan::from_token(
                self.consume(TokenType::RightParen, "expected `)` after expression")?,
            );
            return Ok(Expr::new(
                start.through(end),
                ExprKind::Group(Box::new(expr)),
            ));
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
        SourceSpan::from_token(self.peek())
    }

    fn previous_span(&self) -> SourceSpan {
        SourceSpan::from_token(self.previous())
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

    #[test]
    fn recovering_parse_keeps_valid_statements_after_an_error() {
        let mut scanner = Scanner::new("let first = 1; print(1 + ); let last = 2;");
        let tokens = scanner.scan_tokens().clone();
        let report = Parser::new(tokens).parse_recovering();
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.statements.len(), 2);
        assert!(matches!(
            report.statements[0].kind,
            StatementKind::Let { .. }
        ));
        assert!(matches!(
            report.statements[1].kind,
            StatementKind::Let { .. }
        ));
    }

    #[test]
    fn consecutive_doc_comments_form_one_prose_item() {
        let mut scanner =
            Scanner::new("/// First line\n/// second line\n///\n/// New paragraph\nlet x = 1;");
        let tokens = scanner.scan_tokens().clone();
        let program = Parser::new(tokens).parse().unwrap();
        assert!(matches!(
            &program[0].kind,
            StatementKind::DocComment(text) if text == "First line\nsecond line"
        ));
        assert!(matches!(
            &program[1].kind,
            StatementKind::DocComment(text) if text.is_empty()
        ));
        assert!(matches!(
            &program[2].kind,
            StatementKind::DocComment(text) if text == "New paragraph"
        ));
    }

    #[test]
    fn expressions_and_statements_cover_their_exact_source_ranges() {
        let source = "print(α + 2x);";
        let mut scanner = Scanner::new(source);
        let tokens = scanner.scan_tokens().clone();
        let program = Parser::new(tokens).parse().unwrap();

        assert_eq!(
            &source[program[0].span.byte_start..program[0].span.byte_end],
            source
        );
        let StatementKind::Print(expression) = &program[0].kind else {
            panic!("expected print statement");
        };
        assert_eq!(
            &source[expression.span.byte_start..expression.span.byte_end],
            "α + 2x"
        );
        let ExprKind::Binary { left, right, .. } = &expression.kind else {
            panic!("expected addition");
        };
        assert_eq!(&source[left.span.byte_start..left.span.byte_end], "α");
        assert_eq!(&source[right.span.byte_start..right.span.byte_end], "2x");
        let ExprKind::Binary {
            left: coefficient,
            right: variable,
            ..
        } = &right.kind
        else {
            panic!("expected implicit multiplication");
        };
        assert_eq!(
            &source[coefficient.span.byte_start..coefficient.span.byte_end],
            "2"
        );
        assert_eq!(
            &source[variable.span.byte_start..variable.span.byte_end],
            "x"
        );
    }

    #[test]
    fn transpose_is_a_chainable_postfix_expression() {
        let source = "print(A''[0]);";
        let mut scanner = Scanner::new(source);
        let tokens = scanner.scan_tokens().clone();
        let program = Parser::new(tokens).parse().unwrap();
        let StatementKind::Print(expression) = &program[0].kind else {
            panic!("expected print statement");
        };
        assert_eq!(
            &source[expression.span.byte_start..expression.span.byte_end],
            "A''[0]"
        );
        let ExprKind::Index { target, .. } = &expression.kind else {
            panic!("expected indexing after transpose");
        };
        assert!(matches!(
            target.kind,
            ExprKind::Unary {
                op: UnaryOp::Transpose,
                ..
            }
        ));
    }

    #[test]
    fn matrix_rule_uses_the_single_function_parameter_as_its_size() {
        let mut scanner = Scanner::new("let A(n) = (i + j)_(i,j);");
        let tokens = scanner.scan_tokens().clone();
        let program = Parser::new(tokens).parse().unwrap();
        let StatementKind::Function { params, body, .. } = &program[0].kind else {
            panic!("expected function declaration");
        };
        let FunctionBody::Expression(expression) = body else {
            panic!("expected expression body");
        };
        let ExprKind::Call { args, .. } = &expression.kind else {
            panic!("expected lowered matrix-rule call");
        };
        assert_eq!(args.len(), 4);
        assert!(matches!(args[3].kind, ExprKind::Variable(symbol) if symbol == params[0]));
    }
}
