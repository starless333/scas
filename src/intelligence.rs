//! Lightweight source intelligence shared by the GUI and future protocol clients.
//! Operations are conservative when a name has multiple declarations: rename
//! and definition lookup refuse to guess across shadowed bindings.

use crate::ast::{Expr, ExprKind, Statement, StatementKind};
use crate::interpreter::Interpreter;
use crate::lexer::token_type::TokenType;
use crate::lexer::Scanner;
use crate::parser::Parser;
use crate::Limits;
use std::collections::HashSet;
use std::io;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub name: String,
    pub span: Range<usize>,
    pub line: usize,
    pub column: usize,
    pub is_definition: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionSignature {
    pub name: String,
    pub parameters: Vec<String>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct SourceIndex {
    pub occurrences: Vec<Occurrence>,
    pub functions: Vec<FunctionSignature>,
    scoped_names: HashSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallContext {
    pub name: String,
    pub active_parameter: usize,
    pub parameters: Vec<String>,
}

impl SourceIndex {
    pub fn new(source: &str) -> Self {
        let mut scanner = Scanner::new(source);
        let tokens = scanner.scan_tokens().clone();
        let symbols = scanner.into_symbols();
        // The parser recognizes both `fn f(x)` and mathematical `f(x) = ...`.
        // Use its declaration spans so a call in an equation is never guessed
        // to be a definition by a token-only heuristic.
        let declarations = Parser::new(tokens.clone()).parse_recovering().statements;
        let function_starts: HashSet<usize> = declarations
            .iter()
            .filter_map(|statement| {
                let StatementKind::Function { name, .. } = &statement.kind else {
                    return None;
                };
                tokens
                    .iter()
                    .find(|token| {
                        token.token_type == TokenType::Identifier
                            && token.symbol == *name
                            && token.byte_start >= statement.span.byte_start
                            && token.byte_end <= statement.span.byte_end
                    })
                    .map(|token| token.byte_start)
            })
            .collect();
        let mut index = Self::default();
        let mut block_depth = 0usize;
        for (position, token) in tokens.iter().enumerate() {
            match token.token_type {
                TokenType::LeftBrace | TokenType::RenderLeftBrace => block_depth += 1,
                TokenType::RightBrace | TokenType::RenderRightBrace => {
                    block_depth = block_depth.saturating_sub(1)
                }
                _ => {}
            }
            if token.token_type != TokenType::Identifier {
                continue;
            }
            let name = symbols.resolve(token.symbol).to_owned();
            let is_function = function_starts.contains(&token.byte_start)
                || (position > 0 && tokens[position - 1].token_type == TokenType::Fn);
            let is_definition = is_function
                || (position > 0
                    && matches!(
                        tokens[position - 1].token_type,
                        TokenType::Let | TokenType::Fn
                    ));
            if is_definition && block_depth > 0 {
                index.scoped_names.insert(name.clone());
            }
            index.occurrences.push(Occurrence {
                name: name.clone(),
                span: token.byte_start..token.byte_end,
                line: token.line,
                column: token.column,
                is_definition,
            });
            if is_function {
                let mut parameters = Vec::new();
                if tokens
                    .get(position + 1)
                    .is_some_and(|next| next.token_type == TokenType::LeftParen)
                {
                    let mut cursor = position + 2;
                    while let Some(parameter) = tokens.get(cursor) {
                        if parameter.token_type == TokenType::RightParen {
                            break;
                        }
                        if parameter.token_type == TokenType::Identifier {
                            let parameter_name = symbols.resolve(parameter.symbol).to_owned();
                            index.scoped_names.insert(parameter_name.clone());
                            parameters.push(parameter_name);
                        }
                        cursor += 1;
                    }
                }
                index.functions.push(FunctionSignature {
                    name,
                    parameters,
                    span: token.byte_start..token.byte_end,
                });
            }
        }
        index
    }

    pub fn at(&self, byte: usize) -> Option<&Occurrence> {
        self.occurrences
            .iter()
            .find(|item| item.span.start <= byte && byte < item.span.end)
    }

    pub fn definition(&self, name: &str) -> Option<&Occurrence> {
        // Without a scope graph, references to a parameter cannot safely be
        // distinguished from equally named document-level bindings.
        if self.scoped_names.contains(name) {
            return None;
        }
        let mut definitions = self
            .occurrences
            .iter()
            .filter(|item| item.name == name && item.is_definition);
        let definition = definitions.next()?;
        definitions.next().is_none().then_some(definition)
    }

    pub fn references(&self, name: &str) -> Vec<&Occurrence> {
        if self.definition(name).is_none() {
            return Vec::new();
        }
        self.occurrences
            .iter()
            .filter(|item| item.name == name)
            .collect()
    }

    pub fn rename(&self, name: &str, replacement: &str) -> Option<Vec<Range<usize>>> {
        if !valid_identifier(replacement)
            || self.definition(name).is_none()
            || (name != replacement && self.occurrences.iter().any(|item| item.name == replacement))
        {
            return None;
        }
        Some(
            self.references(name)
                .into_iter()
                .map(|item| item.span.clone())
                .collect(),
        )
    }

    pub fn user_functions(&self) -> impl Iterator<Item = &FunctionSignature> {
        self.functions.iter()
    }

    pub fn call_at(&self, source: &str, byte: usize) -> Option<CallContext> {
        let mut scanner = Scanner::new(source);
        let tokens = scanner.scan_tokens().clone();
        let symbols = scanner.into_symbols();
        let mut stack: Vec<(String, usize)> = Vec::new();
        for (position, token) in tokens.iter().enumerate() {
            if token.byte_start >= byte {
                break;
            }
            match token.token_type {
                TokenType::LeftParen => {
                    let name = position
                        .checked_sub(1)
                        .and_then(|previous| tokens.get(previous))
                        .filter(|previous| previous.token_type == TokenType::Identifier)
                        .map(|previous| symbols.resolve(previous.symbol).to_owned())
                        .unwrap_or_default();
                    stack.push((name, 0));
                }
                TokenType::RightParen => {
                    stack.pop();
                }
                TokenType::Comma => {
                    if let Some((_, active)) = stack.last_mut() {
                        *active += 1;
                    }
                }
                _ => {}
            }
        }
        let (name, active_parameter) =
            stack.into_iter().rev().find(|(name, _)| !name.is_empty())?;
        let parameters = self
            .functions
            .iter()
            .find(|function| function.name == name)
            .map(|function| function.parameters.clone())
            .unwrap_or_else(|| {
                builtin_parameters(&name)
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect()
            });
        Some(CallContext {
            name,
            active_parameter,
            parameters,
        })
    }
}

fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let shape = chars
        .next()
        .is_some_and(|character| character.is_alphabetic() || character == '_')
        && chars.all(|character| character.is_alphanumeric() || character == '_');
    if !shape {
        return false;
    }
    let mut scanner = Scanner::new(name);
    let tokens = scanner.scan_tokens();
    let identifier = tokens.len() == 2 && tokens[0].token_type == TokenType::Identifier;
    identifier && scanner.errors().is_empty()
}

fn builtin_parameters(name: &str) -> &'static [&'static str] {
    match name {
        "derivative" | "integral" | "solve" | "factor" | "expand" | "simplify" => {
            &["expression", "variable"]
        }
        "definite_integral" => &["expression", "variable", "lower", "upper"],
        "rref" | "inverse" | "det" | "transpose" | "trace" => &["matrix"],
        "matmul" => &["left", "right"],
        "print" => &["value"],
        _ => &[],
    }
}

/// Evaluate the smallest expression under a source byte with a bounded,
/// isolated interpreter. Hover never mutates the live REPL or document state.
pub fn evaluate_hover(source: &str, byte: usize) -> Option<(Range<usize>, String)> {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    if !scanner.errors().is_empty() {
        return None;
    }
    let symbols = scanner.into_symbols();
    let statements = Parser::new(tokens).parse_recovering().statements;
    let expression = statements
        .iter()
        .find_map(|statement| expression_in_statement(statement, byte))?;
    let mut interpreter = Interpreter::with_limits(
        symbols,
        io::sink(),
        Limits {
            max_steps: 8_000,
            max_call_depth: 24,
            max_exponent: 2_000,
            max_number_bits: 100_000,
            ..Limits::default()
        },
    );
    // Establish simple bindings before the hovered expression. Skip earlier
    // loops, calls, and render operations; these may have side effects or be
    // too expensive for a tooltip.
    for statement in statements
        .iter()
        .take_while(|statement| statement.span.byte_end <= expression.span.byte_start)
    {
        if matches!(
            statement.kind,
            StatementKind::Let { .. } | StatementKind::Function { .. }
        ) && interpreter.execute_statement(statement).is_err()
        {
            return None;
        }
    }
    let value = interpreter.evaluate_expression(expression).ok()?;
    let result = interpreter.display_value(&value);
    Some((expression.span.byte_start..expression.span.byte_end, result))
}

fn expression_in_statement(statement: &Statement, byte: usize) -> Option<&Expr> {
    let contains =
        |expression: &Expr| expression.span.byte_start <= byte && byte < expression.span.byte_end;
    match &statement.kind {
        StatementKind::Let { value, .. } | StatementKind::Return(value) => value
            .as_ref()
            .filter(|expression| contains(expression))
            .map(|expression| smallest_expression(expression, byte)),
        StatementKind::Expr(expression)
        | StatementKind::Print(expression)
        | StatementKind::Show(expression)
        | StatementKind::RenderLine { expression, .. } => {
            contains(expression).then(|| smallest_expression(expression, byte))
        }
        StatementKind::Assign { value, .. } => {
            contains(value).then(|| smallest_expression(value, byte))
        }
        StatementKind::Function { body, .. } => match body {
            crate::ast::statement::FunctionBody::Expression(expression) => {
                contains(expression).then(|| smallest_expression(expression, byte))
            }
            crate::ast::statement::FunctionBody::Block(statements) => statements
                .iter()
                .find_map(|statement| expression_in_statement(statement, byte)),
        },
        StatementKind::If {
            condition,
            then_branch,
            else_branch,
        } => contains(condition)
            .then(|| smallest_expression(condition, byte))
            .or_else(|| expression_in_statement(then_branch, byte))
            .or_else(|| {
                else_branch
                    .as_ref()
                    .and_then(|statement| expression_in_statement(statement, byte))
            }),
        StatementKind::While { condition, body } => contains(condition)
            .then(|| smallest_expression(condition, byte))
            .or_else(|| expression_in_statement(body, byte)),
        StatementKind::For {
            init,
            condition,
            update,
            body,
        } => expression_in_statement(init, byte)
            .or_else(|| contains(condition).then(|| smallest_expression(condition, byte)))
            .or_else(|| expression_in_statement(update, byte))
            .or_else(|| expression_in_statement(body, byte)),
        StatementKind::Block(statements)
        | StatementKind::RenderBlock(statements)
        | StatementKind::Chain(statements) => statements
            .iter()
            .find_map(|statement| expression_in_statement(statement, byte)),
        StatementKind::Break
        | StatementKind::Continue
        | StatementKind::DocComment(_)
        | StatementKind::Import(_) => None,
    }
}

fn smallest_expression(expression: &Expr, byte: usize) -> &Expr {
    let children: Vec<&Expr> = match &expression.kind {
        ExprKind::Group(inner) | ExprKind::Unary { expr: inner, .. } => vec![inner],
        ExprKind::Array(items) => items.iter().collect(),
        ExprKind::Index { target, index } => vec![target, index],
        ExprKind::Call { callee, args } => std::iter::once(callee.as_ref())
            .chain(args.iter())
            .collect(),
        ExprKind::Binary { left, right, .. } => vec![left, right],
        _ => Vec::new(),
    };
    children
        .into_iter()
        .find(|child| child.span.byte_start <= byte && byte < child.span.byte_end)
        .map_or(expression, |child| smallest_expression(child, byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_references_and_rename_ignore_prose_and_strings() {
        let source = "let α = 2; let beta = 3; /// α\nprint(α); print(\"α\");";
        let index = SourceIndex::new(source);
        let references = index.references("α");
        assert_eq!(references.len(), 2);
        assert_eq!(index.definition("α").unwrap().line, 1);
        assert_eq!(index.rename("α", "gamma").unwrap().len(), 2);
        assert!(index.rename("α", "1bad").is_none());
        assert!(index.rename("α", "let").is_none());
        assert!(index.rename("α", "beta").is_none());
    }

    #[test]
    fn ambiguous_definitions_are_not_renamed() {
        let index = SourceIndex::new("let x = 1; let x = 2; print(x);");
        assert!(index.definition("x").is_none());
        assert!(index.rename("x", "y").is_none());
    }

    #[test]
    fn parameter_shadowing_is_not_renamed_without_scope_resolution() {
        let index = SourceIndex::new("let x = 1; fn f(x) = x + 1; print(x);");
        assert!(index.definition("x").is_none());
        assert!(index.rename("x", "y").is_none());
        let index = SourceIndex::new("let x = 1; if true { let x = 2; } print(x);");
        assert!(index.rename("x", "y").is_none());
    }

    #[test]
    fn signature_tracks_nested_call_arguments() {
        let source = "fn blend(a, b) = a + b; blend(max(1, 2), 4);";
        let index = SourceIndex::new(source);
        let byte = source.find(", 4").unwrap() + 2;
        let call = index.call_at(source, byte).unwrap();
        assert_eq!(call.name, "blend");
        assert_eq!(call.active_parameter, 1);
        assert_eq!(call.parameters, ["a", "b"]);
    }

    #[test]
    fn hover_evaluates_smallest_arithmetic_expression_with_prior_bindings() {
        let source = "let x = 6; print(x * (3 + 4));";
        let byte = source.find('+').unwrap();
        let (span, value) = evaluate_hover(source, byte).unwrap();
        assert_eq!(&source[span], "3 + 4");
        assert_eq!(value, "7");
        let byte = source.find('*').unwrap();
        assert_eq!(evaluate_hover(source, byte).unwrap().1, "42");
    }
    #[test]
    fn shorthand_functions_have_navigable_references() {
        let source = "f(x) = x^2;\nlet y = f(3);\nprint(f(y));";
        let index = SourceIndex::new(source);
        assert_eq!(index.definition("f").unwrap().line, 1);
        assert_eq!(index.references("f").len(), 3);
        assert_eq!(index.user_functions().next().unwrap().parameters, ["x"]);
        assert_eq!(index.rename("f", "square").unwrap().len(), 3);
    }
}
