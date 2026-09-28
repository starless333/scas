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
        let source_line = source
            .split('\n')
            .nth(self.line.saturating_sub(1))
            .unwrap_or("");
        let line_chars: Vec<char> = source_line.chars().collect();
        let start = self.column.saturating_sub(1).min(line_chars.len());
        let available = line_chars.len().saturating_sub(start);
        let marker_width = self.length.max(1).min(available.max(1));
        let indentation: String = line_chars
            .iter()
            .take(start)
            .map(|character| if *character == '\t' { '\t' } else { ' ' })
            .collect();
        let gutter_width = self.line.to_string().len();

        format!(
            "error: {}\n {:>gutter_width$}--> {}:{}:{}\n {:>gutter_width$} |\n {:>gutter_width$} | {}\n {:>gutter_width$} | {}{}",
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
