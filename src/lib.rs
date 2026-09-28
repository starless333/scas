//! Reusable scanner, parser, and interpreter pipeline for SCAS.

pub mod ast;
pub mod interpreter;
pub mod lexer;
pub mod number;
pub mod parser;

use interpreter::Interpreter;
use lexer::{ScanError, Scanner};
use parser::{ParseError, Parser};
use std::fmt;
use std::io::Write;

#[derive(Debug)]
pub enum ExecutionError {
    Scan(Vec<ScanError>),
    Parse(ParseError),
    Runtime(String),
}

impl ExecutionError {
    pub fn render(&self, source: &str, source_name: &str) -> String {
        match self {
            Self::Scan(errors) => errors
                .iter()
                .map(|error| error.render(source, source_name))
                .collect::<Vec<_>>()
                .join("\n"),
            Self::Parse(error) => error.render(source, source_name),
            Self::Runtime(message) => format!("runtime error: {message}"),
        }
    }
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scan(errors) => write!(formatter, "{} lexical error(s)", errors.len()),
            Self::Parse(error) => error.fmt(formatter),
            Self::Runtime(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for ExecutionError {}

/// Scan, parse, and execute a complete source program.
///
/// Language output is written only to `output`; this function never prints or
/// terminates the process, which makes it suitable for tests and embedding.
pub fn execute<W: Write>(source: &str, output: W) -> Result<(), ExecutionError> {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    if !scanner.errors().is_empty() {
        return Err(ExecutionError::Scan(scanner.errors().to_vec()));
    }
    let symbols = scanner.into_symbols();
    let program = Parser::new(tokens).parse().map_err(ExecutionError::Parse)?;
    Interpreter::new(symbols, output)
        .run(&program)
        .map_err(ExecutionError::Runtime)
}
