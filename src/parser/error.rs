use crate::lexer::token::Token;
use crate::lexer::token_type::TokenType;
use std::fmt;

/// A syntax error tied to the token at which parsing could not continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub length: usize,
    pub found: TokenType,
}

impl ParseError {
    pub fn at(token: &Token, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            line: token.line,
            column: token.column,
            length: token.length,
            found: token.token_type,
        }
    }

    /// Render a compact, Rust-style diagnostic with the failing source line.
    pub fn render(&self, source: &str, source_name: &str) -> String {
        let source_line = source
            .split('\n')
            .nth(self.line.saturating_sub(1))
            .unwrap_or("");
        let line_chars: Vec<char> = source_line.chars().collect();
        let start = self.column.saturating_sub(1).min(line_chars.len());
        let available = line_chars.len().saturating_sub(start);
        let marker_width = self.length.max(1).min(available.max(1));
        let gutter_width = self.line.to_string().len();

        let mut indentation = String::new();
        for c in line_chars.iter().take(start) {
            indentation.push(if *c == '\t' { '\t' } else { ' ' });
        }

        let label = if self.found == TokenType::Eof {
            "unexpected end of input".to_owned()
        } else {
            let lexeme: String = line_chars.iter().skip(start).take(marker_width).collect();
            format!("unexpected `{lexeme}`")
        };

        format!(
            "error: {}\n {:>gutter_width$}--> {}:{}:{}\n {:>gutter_width$} |\n {:>gutter_width$} | {}\n {:>gutter_width$} | {}{} {}",
            self.message,
            "",
            source_name,
            self.line,
            self.column,
            "",
            self.line,
            source_line,
            "",
            indentation,
            "^".repeat(marker_width),
            label,
        )
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}:{}",
            self.message, self.line, self.column
        )
    }
}

impl std::error::Error for ParseError {}
