//! Front-end utilities shared by the self-documenter and future IDE clients.

use crate::ast::statement::FunctionBody;
use crate::ast::{AssignOp, Expr, SourceSpan, Statement, StatementKind};
use crate::interpreter::Interpreter;
use crate::latex::LatexRenderer;
use crate::lexer::Scanner;
use crate::parser::Parser;
use crate::{ExecutionError, Limits, RuntimeError};
use std::io;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interpolation {
    /// Byte range covering the complete `#{ ... }` marker.
    pub marker_range: Range<usize>,
    /// Byte range of the expression text inside the marker.
    pub expression_range: Range<usize>,
    pub expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpolationError {
    pub byte_offset: usize,
    pub message: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentKind {
    Prose,
    DisplayMath,
    AlignedMath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentFragment {
    pub span: SourceSpan,
    pub kind: FragmentKind,
    pub latex: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentDiagnostic {
    pub span: SourceSpan,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    pub fragments: Vec<DocumentFragment>,
    pub diagnostics: Vec<DocumentDiagnostic>,
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildOptions {
    pub packages: Vec<String>,
}

impl Document {
    pub fn body(&self) -> String {
        self.fragments
            .iter()
            .map(|fragment| fragment.latex.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn latex(&self) -> String {
        let mut user_packages = String::new();
        for package in &self.packages {
            if !package.is_empty()
                && package
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
            {
                user_packages.push_str(r"\usepackage{");
                user_packages.push_str(package);
                user_packages.push_str("}\n");
            }
        }
        format!(
            "\\documentclass[11pt]{{article}}\n\\usepackage{{amsmath}}\n\\usepackage{{amssymb}}\n\\usepackage{{upgreek}}\n\\usepackage{{booktabs}}\n\\usepackage[margin=1in]{{geometry}}\n{user_packages}\\begin{{document}}\n{}\n\\end{{document}}\n",
            self.body()
        )
    }
}

/// Parse, execute, and assemble one self-documenting SCAS source file.
pub fn build(source: &str) -> Result<Document, ExecutionError> {
    build_with_options(source, BuildOptions::default())
}

pub fn build_with_options(source: &str, options: BuildOptions) -> Result<Document, ExecutionError> {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    if !scanner.errors().is_empty() {
        return Err(ExecutionError::Scan(scanner.errors().to_vec()));
    }
    let symbols = scanner.into_symbols();
    let statements = Parser::new(tokens).parse().map_err(ExecutionError::Parse)?;
    let mut interpreter = Interpreter::with_limits(symbols, io::sink(), Limits::default());
    let mut document = Document {
        packages: options.packages,
        ..Document::default()
    };
    for statement in &statements {
        process_top_level(statement, &mut interpreter, &mut document)?;
    }
    Ok(document)
}

fn process_top_level(
    statement: &Statement,
    interpreter: &mut Interpreter<io::Sink>,
    document: &mut Document,
) -> Result<(), ExecutionError> {
    match &statement.kind {
        StatementKind::DocComment(prose) => {
            let latex = interpolate(prose, statement.span, interpreter, document);
            document.fragments.push(DocumentFragment {
                span: statement.span,
                kind: FragmentKind::Prose,
                latex,
            });
        }
        StatementKind::RenderLine {
            expression,
            show_result,
        } => {
            let source = LatexRenderer::new(interpreter.symbols()).expression(expression);
            let latex = if *show_result {
                match interpreter.evaluate_expression(expression) {
                    Ok(value) => format!(
                        r"\[{source} = {}\]",
                        LatexRenderer::new(interpreter.symbols()).value(&value)
                    ),
                    Err(error) => {
                        push_runtime_diagnostic(error, statement.span, document);
                        format!(r"\[{source} = \text{{\textbf{{?}}}}\]")
                    }
                }
            } else {
                interpreter
                    .evaluate_expression(expression)
                    .map_err(ExecutionError::Runtime)?;
                format!(r"\[{source}\]")
            };
            document.fragments.push(DocumentFragment {
                span: statement.span,
                kind: FragmentKind::DisplayMath,
                latex,
            });
        }
        StatementKind::RenderBlock(statements) => {
            render_block(statements, statement.span, interpreter, document)?;
        }
        _ => interpreter
            .execute_statement(statement)
            .map_err(ExecutionError::Runtime)?,
    }
    Ok(())
}

fn render_block(
    statements: &[Statement],
    span: SourceSpan,
    interpreter: &mut Interpreter<io::Sink>,
    document: &mut Document,
) -> Result<(), ExecutionError> {
    let mut rows = Vec::new();
    for statement in statements {
        let renderer = LatexRenderer::new(interpreter.symbols());
        let row = match &statement.kind {
            StatementKind::DocComment(prose) => Some(format!(
                r"\intertext{{{}}}",
                interpolate(prose, statement.span, interpreter, document)
            )),
            StatementKind::Expr(expression) => {
                let text = renderer.expression(expression);
                interpreter
                    .execute_statement(statement)
                    .map_err(ExecutionError::Runtime)?;
                Some(text)
            }
            StatementKind::Show(expression) => {
                let text = renderer.expression(expression);
                Some(match interpreter.evaluate_expression(expression) {
                    Ok(value) => format!(
                        "{text} &= {}",
                        LatexRenderer::new(interpreter.symbols()).value(&value)
                    ),
                    Err(error) => {
                        push_runtime_diagnostic(error, statement.span, document);
                        format!(r"{text} &= \text{{\textbf{{?}}}}")
                    }
                })
            }
            StatementKind::Let { name, value } => {
                let row = value.as_ref().map(|value| {
                    let name = renderer.identifier(interpreter.symbols().resolve(*name));
                    format!("{} &= {}", name, renderer.expression(value))
                });
                interpreter
                    .execute_statement(statement)
                    .map_err(ExecutionError::Runtime)?;
                row
            }
            StatementKind::Assign { name, op, value } => {
                let name = renderer.identifier(interpreter.symbols().resolve(*name));
                let rhs = renderer.expression(value);
                let shown = match op {
                    AssignOp::Assign => rhs,
                    _ => format!("{name} {} {rhs}", assign_operator(*op)),
                };
                interpreter
                    .execute_statement(statement)
                    .map_err(ExecutionError::Runtime)?;
                Some(format!("{name} &= {shown}"))
            }
            StatementKind::Function {
                name,
                params,
                body: FunctionBody::Expression(body),
            } => {
                let name = interpreter.symbols().resolve(*name).to_owned();
                let params = params
                    .iter()
                    .map(|symbol| renderer.identifier(interpreter.symbols().resolve(*symbol)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let body = renderer.expression(body);
                interpreter
                    .execute_statement(statement)
                    .map_err(ExecutionError::Runtime)?;
                Some(format!(
                    r"\operatorname{{{name}}}\left({params}\right) &= {body}"
                ))
            }
            StatementKind::RenderLine { .. } | StatementKind::RenderBlock(_) => {
                process_top_level(statement, interpreter, document)?;
                None
            }
            StatementKind::If { .. } | StatementKind::While { .. } | StatementKind::For { .. } => {
                document.diagnostics.push(DocumentDiagnostic {
                    span: statement.span,
                    severity: DiagnosticSeverity::Info,
                    message: "control flow executed silently inside render block".to_owned(),
                });
                interpreter
                    .execute_statement(statement)
                    .map_err(ExecutionError::Runtime)?;
                None
            }
            _ => {
                interpreter
                    .execute_statement(statement)
                    .map_err(ExecutionError::Runtime)?;
                None
            }
        };
        if let Some(row) = row {
            rows.push(row);
        }
    }
    if !rows.is_empty() {
        document.fragments.push(DocumentFragment {
            span,
            kind: FragmentKind::AlignedMath,
            latex: format!(r"\begin{{align*}}{}\end{{align*}}", rows.join(" \\\n")),
        });
    }
    Ok(())
}

fn assign_operator(operator: AssignOp) -> &'static str {
    match operator {
        AssignOp::Assign => "=",
        AssignOp::AddAssign => "+",
        AssignOp::SubAssign => "-",
        AssignOp::MulAssign => r"\cdot",
        AssignOp::DivAssign => "/",
        AssignOp::ModAssign => r"\bmod",
        AssignOp::PowAssign => "^",
        AssignOp::BitAndAssign => r"\mathbin{\&}",
        AssignOp::BitOrAssign => r"\mathbin{|}",
    }
}

fn interpolate(
    prose: &str,
    span: SourceSpan,
    interpreter: &mut Interpreter<io::Sink>,
    document: &mut Document,
) -> String {
    let Ok(found) = interpolations(prose) else {
        document.diagnostics.push(DocumentDiagnostic {
            span,
            severity: DiagnosticSeverity::Error,
            message: "unterminated prose interpolation".to_owned(),
        });
        return prose.to_owned();
    };
    let mut output = String::new();
    let mut cursor = 0;
    for item in found {
        output.push_str(&prose[cursor..item.marker_range.start]);
        match parse_embedded_expression(&item.expression, interpreter) {
            Ok(expression) => match interpreter.evaluate_expression(&expression) {
                Ok(value) => {
                    output.push_str(&LatexRenderer::new(interpreter.symbols()).value(&value))
                }
                Err(error) => {
                    push_runtime_diagnostic(error, span, document);
                    output.push_str(r"\text{\textbf{?}}");
                }
            },
            Err(message) => {
                document.diagnostics.push(DocumentDiagnostic {
                    span,
                    severity: DiagnosticSeverity::Error,
                    message,
                });
                output.push_str(r"\text{\textbf{?}}");
            }
        }
        cursor = item.marker_range.end;
    }
    output.push_str(&prose[cursor..]);
    output
}

fn parse_embedded_expression(
    source: &str,
    interpreter: &mut Interpreter<io::Sink>,
) -> Result<Expr, String> {
    let wrapped = format!("print({source});");
    let mut scanner = Scanner::with_symbols(&wrapped, interpreter.symbols().clone());
    let tokens = scanner.scan_tokens().clone();
    if let Some(error) = scanner.errors().first() {
        return Err(error.to_string());
    }
    let symbols = scanner.into_symbols();
    let parsed = Parser::new(tokens)
        .parse()
        .map_err(|error| error.to_string())?;
    interpreter.extend_symbols(symbols);
    match parsed.into_iter().next().map(|statement| statement.kind) {
        Some(StatementKind::Print(expression)) => Ok(expression),
        _ => Err("interpolation must contain one expression".to_owned()),
    }
}

fn push_runtime_diagnostic(error: RuntimeError, fallback: SourceSpan, document: &mut Document) {
    document.diagnostics.push(DocumentDiagnostic {
        span: error.span.unwrap_or(fallback),
        severity: DiagnosticSeverity::Error,
        message: error.message,
    });
}

/// Locate prose interpolations while respecting nested braces and strings.
pub fn interpolations(prose: &str) -> Result<Vec<Interpolation>, InterpolationError> {
    let bytes = prose.as_bytes();
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor + 1 < bytes.len() {
        if bytes[cursor] != b'#' || bytes[cursor + 1] != b'{' {
            cursor += 1;
            continue;
        }
        let marker_start = cursor;
        let expression_start = cursor + 2;
        cursor = expression_start;
        let mut depth = 1usize;
        let mut in_string = false;
        let mut escaped = false;
        while cursor < bytes.len() {
            let byte = bytes[cursor];
            if in_string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
            } else if byte == b'"' {
                in_string = true;
            } else if byte == b'{' {
                depth += 1;
            } else if byte == b'}' {
                depth -= 1;
                if depth == 0 {
                    let expression_end = cursor;
                    cursor += 1;
                    result.push(Interpolation {
                        marker_range: marker_start..cursor,
                        expression_range: expression_start..expression_end,
                        expression: prose[expression_start..expression_end].trim().to_owned(),
                    });
                    break;
                }
            }
            cursor += 1;
        }
        if depth != 0 {
            return Err(InterpolationError {
                byte_offset: marker_start,
                message: "unterminated prose interpolation",
            });
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_search_is_brace_depth_and_string_aware() {
        let prose = r#"Value #{ f(x)[0] } and #{ "}" + g({x}) }."#;
        let found = interpolations(prose).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].expression, "f(x)[0]");
        assert_eq!(found[1].expression, r#""}" + g({x})"#);
        for item in found {
            assert!(prose[item.marker_range].starts_with("#{"));
        }
    }

    #[test]
    fn unterminated_interpolation_is_a_structured_error() {
        let error = interpolations("value #{ x + 1").unwrap_err();
        assert_eq!(error.byte_offset, 6);
    }

    #[test]
    fn builds_source_order_fragments_with_evaluated_results_and_interpolation() {
        let source = "let x = 2;\n/// Current value: #{ x }\n\\[ x + 3 => \\]\n\\{\nlet y = x * 4;\ny / 2 =>;\n\\}\n";
        let document = build(source).unwrap();
        assert_eq!(document.fragments.len(), 3);
        assert!(document.fragments[0].latex.contains("Current value: 2"));
        assert!(document.fragments[1].latex.contains(r"\mathrm{x} + 3 = 5"));
        assert!(document.fragments[2]
            .latex
            .contains(r"\mathrm{y} &= \mathrm{x} \cdot 4"));
        assert!(document.fragments[2]
            .latex
            .contains(r"\frac{\mathrm{y}}{2} &= 4"));
        assert!(document.latex().contains(r"\usepackage{amsmath}"));
        assert!(document
            .fragments
            .iter()
            .all(|fragment| fragment.span.line > 0));
    }

    #[test]
    fn interpolation_errors_are_visible_and_non_fatal() {
        let document = build("/// Result: #{ missing(1) }\n").unwrap();
        assert_eq!(document.diagnostics.len(), 1);
        assert!(document.fragments[0].latex.contains(r"\text{\textbf{?}}"));
    }

    #[test]
    fn preamble_accepts_safe_user_packages() {
        let document = build_with_options(
            "/// Hello\n",
            BuildOptions {
                packages: vec!["mathtools".to_owned(), "bad{package}".to_owned()],
            },
        )
        .unwrap();
        let latex = document.latex();
        assert!(latex.contains(r"\usepackage{mathtools}"));
        assert!(!latex.contains("bad{package}"));
    }
}
