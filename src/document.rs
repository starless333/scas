//! Front-end utilities shared by the self-documenter and future IDE clients.

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
}
