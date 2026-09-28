use crate::lexer::Token;

/// A half-open source range with a human-readable start position.
///
/// Byte offsets support editor navigation and slicing. `line`, `column`, and
/// `length` remain Unicode-scalar based for terminal diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceSpan {
    pub line: usize,
    pub column: usize,
    pub length: usize,
    pub byte_start: usize,
    pub byte_end: usize,
}

impl SourceSpan {
    pub const fn diagnostic(line: usize, column: usize, length: usize) -> Self {
        Self {
            line,
            column,
            length,
            byte_start: 0,
            byte_end: 0,
        }
    }

    pub fn from_token(token: &Token) -> Self {
        Self {
            line: token.line,
            column: token.column,
            length: token.length,
            byte_start: token.byte_start,
            byte_end: token.byte_end,
        }
    }

    /// Return the smallest span beginning at `self` and ending at `end`.
    pub fn through(self, end: Self) -> Self {
        let length = if self.line == end.line {
            end.column
                .saturating_add(end.length)
                .saturating_sub(self.column)
        } else {
            self.length
        };
        Self {
            length,
            byte_end: end.byte_end.max(self.byte_end),
            ..self
        }
    }
}
