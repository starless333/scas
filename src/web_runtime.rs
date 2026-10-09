//! Host-independent requests used by the browser Web Worker.
//! No filesystem, process, or thread API is used here.

use crate::ast::StatementKind;
use crate::document::{self, FragmentKind};
use crate::interpreter::Interpreter;
use crate::lexer::Scanner;
use crate::parser::Parser;
use crate::plot;
use crate::{execute_with_options, ExecutionOptions, Limits};
use serde_json::{json, Value};
use std::io;
use std::num::NonZeroU64;
use std::path::Path;

pub const STARTER: &str =
    "/// A live, typeset result\nlet angle = pi / 6;\n\\[ sin(angle) => \\]\nprint(sin(angle));\n";
pub const MIT_EXAMPLE: &str = include_str!("../examples/mit-18-06-session-22.scas");

pub fn browser_limits() -> Limits {
    Limits {
        max_source_chars: 120_000,
        max_tokens: 40_000,
        max_steps: 120_000,
        max_call_depth: 96,
        max_exponent: 10_000,
        max_shift: 100_000,
        max_decimal_precision: 512,
        max_transcendental_precision: 256,
        max_number_bits: 1_000_000,
        ..Limits::default()
    }
}

/// Input and output are JSON strings to keep the worker boundary small and
/// preserve generation IDs when a newer edit supersedes an older evaluation.
pub fn process_request(request: &str) -> String {
    let parsed: Value = match serde_json::from_str(request) {
        Ok(value) => value,
        Err(error) => return json!({ "generation": 0, "error": error.to_string() }).to_string(),
    };
    let generation = parsed["generation"].as_u64().unwrap_or(0);
    let source = parsed["source"].as_str().unwrap_or("");
    if source.chars().count() > browser_limits().max_source_chars {
        return json!({ "generation": generation, "error": "Source exceeds the browser size limit" }).to_string();
    }
    match parsed["kind"].as_str().unwrap_or("run") {
        "plot" => plot_request(
            source,
            parsed["expression"].as_str().unwrap_or(""),
            generation,
        ),
        _ => run_request(source, generation),
    }
}

fn run_request(source: &str, generation: u64) -> String {
    let mut diagnostics = source_diagnostics(source);
    let mut output = Vec::new();
    let execution = execute_with_options(
        source,
        &mut output,
        ExecutionOptions {
            limits: browser_limits(),
            decimal_precision: NonZeroU64::new(34).unwrap(),
        },
    );
    let output = String::from_utf8_lossy(&output).into_owned();
    let runtime_error = execution
        .err()
        .map(|error| error.render(source, "browser.scas"));
    match document::build_live_with_limits(source, browser_limits()) {
        Ok(document) => {
            let fragments = document
                .fragments
                .iter()
                .map(|fragment| {
                    json!({
                        "kind": match fragment.kind {
                            FragmentKind::Prose => "prose",
                            FragmentKind::DisplayMath => "display",
                            FragmentKind::AlignedMath => "aligned",
                        },
                        "latex": fragment.latex,
                        "span": {
                            "line": fragment.span.line,
                            "column": fragment.span.column,
                            "length": fragment.span.length,
                            "byte_start": fragment.span.byte_start,
                            "byte_end": fragment.span.byte_end,
                        }
                    })
                })
                .collect::<Vec<_>>();
            diagnostics.extend(document.diagnostics.iter().map(|diagnostic| {
                json!({
                    "line": diagnostic.span.line,
                    "column": diagnostic.span.column,
                    "message": diagnostic.message,
                })
            }));
            json!({
                "generation": generation,
                "output": output,
                "error": runtime_error,
                "tex": document.latex(),
                "fragments": fragments,
                "diagnostics": diagnostics,
            })
            .to_string()
        }
        Err(error) => json!({
            "generation": generation,
            "output": output,
            "error": runtime_error.unwrap_or_else(|| error.render(source, "browser.scas")),
            "fragments": [],
            "diagnostics": diagnostics,
        })
        .to_string(),
    }
}

fn source_diagnostics(source: &str) -> Vec<Value> {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    let mut diagnostics = scanner
        .errors()
        .iter()
        .map(|error| {
            json!({
                "line": error.line,
                "column": error.column,
                "message": error.message,
            })
        })
        .collect::<Vec<_>>();
    if diagnostics.is_empty() {
        diagnostics.extend(
            Parser::new(tokens)
                .parse_recovering()
                .errors
                .into_iter()
                .map(|error| {
                    json!({
                        "line": error.line,
                        "column": error.column,
                        "message": error.message,
                    })
                }),
        );
    }
    diagnostics.truncate(100);
    diagnostics
}

fn plot_request(source: &str, expression: &str, generation: u64) -> String {
    if expression.contains('>') {
        return json!({ "generation": generation, "error": "Browser plots cannot write files; use the SVG download action" }).to_string();
    }
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    if !scanner.errors().is_empty() {
        return json!({ "generation": generation, "error": "Fix source errors before plotting" })
            .to_string();
    }
    let mut symbols = scanner.into_symbols();
    let report = Parser::new(tokens).parse_recovering();
    let mut interpreter = Interpreter::with_limits(symbols.clone(), io::sink(), browser_limits());
    for statement in &report.statements {
        if matches!(
            statement.kind,
            StatementKind::Let { .. } | StatementKind::Function { .. }
        ) {
            if let Err(error) = interpreter.execute_statement(statement) {
                return json!({ "generation": generation, "error": error.message }).to_string();
            }
        }
    }
    match plot::execute_plot_command(
        &mut interpreter,
        &mut symbols,
        expression,
        64,
        17,
        Path::new(""),
    ) {
        Ok(outcome) => {
            let svg = plot::render_svg(&outcome.series, &outcome.bounds, 800, 520);
            let series = outcome
                .series
                .iter()
                .map(|series| {
                    json!({
                        "label": series.label,
                        "samples": series.samples,
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "generation": generation,
                "series": series,
                "bounds": [outcome.bounds.x_min, outcome.bounds.x_max, outcome.bounds.y_min, outcome.bounds.y_max],
                "svg": svg,
            }).to_string()
        }
        Err(error) => json!({ "generation": generation, "error": error }).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_run_produces_output_and_math_without_host_io() {
        let response: Value = serde_json::from_str(&process_request(
            &json!({
                "generation": 7, "source": "let x = 2; \\[ x + 3 => \\] print(x);"
            })
            .to_string(),
        ))
        .unwrap();
        assert_eq!(response["generation"], 7);
        assert!(response["output"].as_str().unwrap().contains('2'));
        assert!(response["fragments"].as_array().unwrap().len() > 0);
    }

    #[test]
    fn browser_plot_returns_svg_and_points() {
        let response: Value = serde_json::from_str(&process_request(
            &json!({
                "generation": 3, "kind": "plot", "source": "let k = 2;", "expression": "k*x"
            })
            .to_string(),
        ))
        .unwrap();
        assert!(response["svg"].as_str().unwrap().contains("<svg"));
        assert!(
            response["series"].as_array().unwrap()[0]["samples"]
                .as_array()
                .unwrap()
                .len()
                > 100
        );
    }

    #[test]
    fn browser_reports_source_locations_for_syntax_errors() {
        let response: Value = serde_json::from_str(&process_request(
            &json!({ "generation": 9, "source": "let x = ;\nprint(1);" }).to_string(),
        ))
        .unwrap();
        let problems = response["diagnostics"].as_array().unwrap();
        assert!(!problems.is_empty(), "{response}");
        assert_eq!(problems[0]["line"], 1);
        assert!(problems[0]["column"].as_u64().unwrap() > 0);
    }

    #[test]
    fn mit_example_builds_in_browser_worker_budget() {
        let response: Value = serde_json::from_str(&process_request(
            &json!({ "generation": 10, "source": MIT_EXAMPLE }).to_string(),
        ))
        .unwrap();
        assert!(response["error"].is_null(), "{response}");
        assert!(response["fragments"].as_array().unwrap().len() > 20);
    }
}
