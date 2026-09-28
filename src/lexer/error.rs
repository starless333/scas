use crate::ast::SourceSpan;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub length: usize,
}

impl ScanError {
    pub fn new(message: impl Into<String>, line: usize, column: usize, length: usize) -> Self {
        Self {
            message: message.into(),
            line,
            column,
            length,
        }
    }

    pub fn render(&self, source: &str, source_name: &str) -> String {
        crate::render_diagnostic(
            "lexical",
            &self.message,
            SourceSpan::diagnostic(self.line, self.column, self.length),
            source,
            source_name,
            None,
        )
    }
}

impl fmt::Display for ScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}:{}",
            self.message, self.line, self.column
        )
    }
}

impl std::error::Error for ScanError {}
