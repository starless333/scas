//! Reusable scanner, parser, and interpreter pipeline for SCAS.

pub mod ast;
pub mod document;
pub mod interpreter;
pub mod latex;
pub mod lexer;
pub mod number;
pub mod parser;

use ast::SourceSpan;
use interpreter::Interpreter;
use lexer::{ScanError, Scanner};
use parser::{ParseError, Parser};
use std::fmt;
use std::io::Write;
use std::num::NonZeroU64;

/// Resource budget for one program execution. All limits are deterministic;
/// exceeding one returns a normal `limit error` rather than panicking or
/// attempting an unbounded allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_source_chars: usize,
    pub max_tokens: usize,
    pub max_nesting: usize,
    pub max_steps: u64,
    pub max_call_depth: usize,
    pub max_exponent: u32,
    pub max_shift: usize,
    pub max_decimal_precision: u64,
    /// Maximum significant-digit precision accepted by transcendental and
    /// special-function algorithms. These operations are substantially more
    /// expensive than ordinary decimal arithmetic, so they have a separate
    /// deterministic budget.
    pub max_transcendental_precision: u64,
    pub max_decimal_scale: u64,
    pub max_number_bits: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_source_chars: 1_000_000,
            max_tokens: 250_000,
            max_nesting: 256,
            max_steps: 1_000_000,
            max_call_depth: 256,
            max_exponent: 100_000,
            max_shift: 1_000_000,
            max_decimal_precision: 10_000,
            max_transcendental_precision: 2_000,
            max_decimal_scale: 100_000,
            max_number_bits: 8_000_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionOptions {
    pub limits: Limits,
    pub decimal_precision: NonZeroU64,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            decimal_precision: NonZeroU64::new(100).expect("100 is non-zero"),
        }
    }
}

#[derive(Debug)]
pub enum ExecutionError {
    Scan(Vec<ScanError>),
    Parse(ParseError),
    Runtime(RuntimeError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub message: String,
    pub span: Option<SourceSpan>,
    pub is_limit: bool,
}

impl RuntimeError {
    pub fn new(message: impl Into<String>, span: Option<SourceSpan>) -> Self {
        Self {
            message: message.into(),
            span,
            is_limit: false,
        }
    }

    pub fn limit(message: impl Into<String>, span: Option<SourceSpan>) -> Self {
        Self {
            message: message.into(),
            span,
            is_limit: true,
        }
    }

    pub fn render(&self, source: &str, source_name: &str) -> String {
        let category = if self.is_limit { "limit" } else { "runtime" };
        let Some(span) = self.span else {
            return format!("{category} error: {}", self.message);
        };
        render_diagnostic(category, &self.message, span, source, source_name, None)
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for RuntimeError {}

impl ExecutionError {
    pub fn render(&self, source: &str, source_name: &str) -> String {
        match self {
            Self::Scan(errors) => errors
                .iter()
                .map(|error| error.render(source, source_name))
                .collect::<Vec<_>>()
                .join("\n"),
            Self::Parse(error) => error.render(source, source_name),
            Self::Runtime(error) => error.render(source, source_name),
        }
    }
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scan(errors) => write!(formatter, "{} lexical error(s)", errors.len()),
            Self::Parse(error) => error.fmt(formatter),
            Self::Runtime(error) => error.message.fmt(formatter),
        }
    }
}

/// Shared rendering used by every diagnostic phase.
pub(crate) fn render_diagnostic(
    category: &str,
    message: &str,
    span: SourceSpan,
    source: &str,
    source_name: &str,
    label: Option<&str>,
) -> String {
    let source_line = source
        .split('\n')
        .nth(span.line.saturating_sub(1))
        .unwrap_or("");
    let chars: Vec<char> = source_line.chars().collect();
    let start = span.column.saturating_sub(1).min(chars.len());
    let width = span
        .length
        .max(1)
        .min(chars.len().saturating_sub(start).max(1));
    let indentation: String = chars[..start]
        .iter()
        .map(|character| if *character == '\t' { '\t' } else { ' ' })
        .collect();
    let gutter = span.line.to_string().len();
    let suffix = label.map(|text| format!(" {text}")).unwrap_or_default();
    format!(
        "{category} error: {message}\n {:>gutter$}--> {source_name}:{}:{}\n {:>gutter$} |\n {:>gutter$} | {source_line}\n {:>gutter$} | {indentation}{}{suffix}",
        "", span.line, span.column, "", span.line, "", "^".repeat(width),
    )
}

impl std::error::Error for ExecutionError {}

/// Scan, parse, and execute a complete source program.
///
/// Language output is written only to `output`; this function never prints or
/// terminates the process, which makes it suitable for tests and embedding.
pub fn execute<W: Write>(source: &str, output: W) -> Result<(), ExecutionError> {
    execute_with_options(source, output, ExecutionOptions::default())
}

pub fn execute_with_limits<W: Write>(
    source: &str,
    output: W,
    limits: Limits,
) -> Result<(), ExecutionError> {
    let precision = Limits::default()
        .max_decimal_precision
        .min(limits.max_decimal_precision)
        .max(1);
    execute_with_options(
        source,
        output,
        ExecutionOptions {
            limits,
            decimal_precision: NonZeroU64::new(precision).expect("precision is at least one"),
        },
    )
}

pub fn execute_with_options<W: Write>(
    source: &str,
    output: W,
    options: ExecutionOptions,
) -> Result<(), ExecutionError> {
    let limits = options.limits;
    if source.chars().count() > limits.max_source_chars {
        return Err(ExecutionError::Runtime(RuntimeError::limit(
            format!(
                "source exceeds the {} character limit",
                limits.max_source_chars
            ),
            Some(SourceSpan::diagnostic(1, 1, 1)),
        )));
    }
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    if !scanner.errors().is_empty() {
        return Err(ExecutionError::Scan(scanner.errors().to_vec()));
    }
    if tokens.len() > limits.max_tokens {
        return Err(ExecutionError::Runtime(RuntimeError::limit(
            format!("program exceeds the {} token limit", limits.max_tokens),
            tokens.last().map(SourceSpan::from_token),
        )));
    }
    let mut nesting = 0usize;
    for token in &tokens {
        use lexer::TokenType::{
            LeftBrace, LeftBracket, LeftParen, RightBrace, RightBracket, RightParen,
        };
        match token.token_type {
            LeftBrace | LeftBracket | LeftParen => {
                nesting += 1;
                if nesting > limits.max_nesting {
                    return Err(ExecutionError::Runtime(RuntimeError::limit(
                        format!("input exceeds the nesting limit of {}", limits.max_nesting),
                        Some(SourceSpan::from_token(token)),
                    )));
                }
            }
            RightBrace | RightBracket | RightParen => nesting = nesting.saturating_sub(1),
            _ => {}
        }
    }
    let symbols = scanner.into_symbols();
    let program = Parser::new(tokens).parse().map_err(ExecutionError::Parse)?;
    let mut interpreter = Interpreter::with_limits(symbols, output, limits);
    interpreter
        .set_decimal_precision(options.decimal_precision)
        .map_err(ExecutionError::Runtime)?;
    interpreter.run(&program).map_err(ExecutionError::Runtime)
}
