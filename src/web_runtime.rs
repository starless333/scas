//! Host-independent requests used by the browser Web Worker.
//! No filesystem, process, or thread API is used here.

use crate::ast::statement::FunctionBody;
use crate::ast::StatementKind;
use crate::document::{self, FragmentKind};
use crate::interpreter::Interpreter;
use crate::latex::LatexRenderer;
use crate::lexer::{Scanner, SymbolTable};
use crate::parser::Parser;
use crate::plot;
use crate::{execute_with_options, ExecutionOptions, Limits};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::io;
use std::num::NonZeroU64;

pub const STARTER: &str =
    "/// A live, typeset result\nlet angle = π / 6;\n\\[ sin(angle) => \\]\nprint(sin(angle));\n";
pub const MIT_EXAMPLE: &str = include_str!("../examples/mit-18-06-session-22.scas");

struct BrowserRepl {
    symbols: SymbolTable,
    interpreter: Interpreter<Vec<u8>>,
    history: Vec<String>,
}

struct BrowserReplResult {
    output: String,
    printed: String,
    error: Option<String>,
    input_latex: Option<String>,
    result_latex: Option<String>,
    plot: Option<Value>,
    clear: bool,
}

impl BrowserReplResult {
    fn message(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            printed: String::new(),
            error: None,
            input_latex: None,
            result_latex: None,
            plot: None,
            clear: false,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            output: String::new(),
            printed: String::new(),
            error: Some(message.into()),
            input_latex: None,
            result_latex: None,
            plot: None,
            clear: false,
        }
    }
}

impl BrowserRepl {
    fn new() -> Self {
        let symbols = SymbolTable::new();
        let interpreter = Interpreter::with_limits(symbols.clone(), Vec::new(), browser_limits());
        Self {
            symbols,
            interpreter,
            history: Vec::new(),
        }
    }

    fn submit(&mut self, input: &str, numerical: bool) -> BrowserReplResult {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return BrowserReplResult::message("");
        }
        if trimmed.chars().count() > browser_limits().max_source_chars {
            return BrowserReplResult::error("REPL input exceeds the browser size limit");
        }
        self.history.push(trimmed.to_owned());
        match trimmed {
            ":reset" => {
                *self = Self::new();
                return BrowserReplResult::message("Session reset");
            }
            ":help" => return BrowserReplResult::message("Commands:\n  :help             show this help\n  :precision N      set decimal precision\n  :plot EXPR        draw a graph (use :plot for examples)\n  :definitions      list definitions\n  :history          show command history\n  :clear            clear displayed history\n  :reset            reset definitions and precision\n\nEnter evaluates. Ctrl+Enter evaluates numerically."),
            ":definitions" => return BrowserReplResult::message(self.interpreter.definition_names().join("\n")),
            ":history" => return BrowserReplResult::message(self.history.iter().enumerate().map(|(index, entry)| format!("{}  {}", index + 1, entry)).collect::<Vec<_>>().join("\n")),
            ":clear" => return BrowserReplResult { clear: true, ..BrowserReplResult::message("") },
            ":precision" => return BrowserReplResult::message(format!("decimal precision: {} digits\nusage: :precision N (N must be greater than zero)", self.interpreter.decimal_precision())),
            _ => {}
        }
        if let Some(suffix) = trimmed.strip_prefix(":plot") {
            let rest = if suffix.is_empty() || suffix.starts_with(char::is_whitespace) {
                Some(suffix.trim())
            } else {
                suffix
                    .strip_prefix('(')
                    .and_then(|source| source.strip_suffix(')'))
                    .map(str::trim)
            };
            if let Some(rest) = rest {
                return self.plot(rest);
            }
        }
        if let Some(argument) = trimmed.strip_prefix(":precision ") {
            return match argument
                .trim()
                .parse::<u64>()
                .ok()
                .and_then(NonZeroU64::new)
            {
                Some(precision) => match self.interpreter.set_decimal_precision(precision) {
                    Ok(()) => BrowserReplResult::message(format!(
                        "decimal precision set to {} digits",
                        self.interpreter.decimal_precision()
                    )),
                    Err(error) => BrowserReplResult::error(error.message),
                },
                None => {
                    BrowserReplResult::error("usage: :precision N (N must be greater than zero)")
                }
            };
        }
        if trimmed.starts_with(':') {
            return BrowserReplResult::error(format!(
                "Unknown command: {trimmed}. Use :help for supported commands."
            ));
        }
        let execution_input = if numerical {
            format!("evalf({trimmed})")
        } else {
            input.to_owned()
        };
        let submission = if execution_input.trim_end().ends_with(';')
            || execution_input.trim_end().ends_with('}')
        {
            execution_input
        } else {
            format!("{execution_input};")
        };
        let mut scanner = Scanner::with_symbols(&submission, self.symbols.clone());
        let tokens = scanner.scan_tokens().clone();
        if let Some(error) = scanner.errors().first() {
            return BrowserReplResult::error(error.render(&submission, "<repl>"));
        }
        if tokens.len() > browser_limits().max_tokens {
            return BrowserReplResult::error("REPL input exceeds the browser token limit");
        }
        let symbols = scanner.into_symbols();
        let program = match Parser::new(tokens).parse() {
            Ok(program) => program,
            Err(error) => return BrowserReplResult::error(error.render(&submission, "<repl>")),
        };
        let input_latex = if numerical {
            repl_source_latex(input, &self.symbols)
        } else {
            repl_program_latex(&program, &symbols)
        };
        self.symbols = symbols.clone();
        self.interpreter.set_symbols(symbols);
        let result = self.interpreter.run_interactive_value(&program);
        let printed =
            String::from_utf8_lossy(&std::mem::take(self.interpreter.output_mut())).into_owned();
        match result {
            Ok(Some(value)) => {
                let output = self.interpreter.display_value(&value);
                let result_latex = LatexRenderer::new(self.interpreter.symbols()).value(&value);
                BrowserReplResult {
                    output,
                    printed,
                    error: None,
                    input_latex,
                    result_latex: Some(result_latex),
                    plot: None,
                    clear: false,
                }
            }
            Ok(None) => BrowserReplResult {
                output: String::new(),
                printed,
                error: None,
                input_latex,
                result_latex: None,
                plot: None,
                clear: false,
            },
            Err(error) => {
                if error.message.starts_with("input required: ") {
                    self.history.pop();
                }
                BrowserReplResult {
                    output: String::new(),
                    printed,
                    error: Some(error.render(&submission, "<repl>")),
                    input_latex,
                    result_latex: None,
                    plot: None,
                    clear: false,
                }
            }
        }
    }

    fn plot(&mut self, rest: &str) -> BrowserReplResult {
        if rest.is_empty() {
            return BrowserReplResult::message("usage: :plot [terminal|svg] EXPRESSION[, EXPRESSION…] [from A to B] [> FILE]\n\nExamples:\n  :plot sin(x)\n  :plot sin(x), cos(x) from -6 to 6\n  :plot svg sin(x) > graph.svg\n\nIn the browser, > FILE offers a download button.");
        }
        let mut symbols = self.symbols.clone();
        let result = plot::execute_plot_command_in_memory(
            &mut self.interpreter,
            &mut symbols,
            rest,
            64,
            18,
            None,
        );
        self.symbols = symbols.clone();
        self.interpreter.set_symbols(symbols);
        match result {
            Ok((outcome, redirect)) => {
                let download_name = redirect.as_deref().and_then(|path| {
                    let name = path.rsplit(['/', '\\']).next()?.trim();
                    (!name.is_empty() && name != "." && name != "..").then(|| name.to_owned())
                });
                let vector = rest == "svg"
                    || rest.starts_with("svg ")
                    || download_name
                        .as_ref()
                        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".svg"));
                let svg = outcome
                    .svg
                    .clone()
                    .unwrap_or_else(|| plot::render_svg(&outcome.series, &outcome.bounds, 800, 520));
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
                let bounds = outcome.bounds;
                let mut result = BrowserReplResult::message("");
                result.plot = Some(json!({
                    "series": series,
                    "bounds": [bounds.x_min, bounds.x_max, bounds.y_min, bounds.y_max],
                    "ascii": outcome.ascii,
                    "svg": svg,
                    "vector": vector,
                    "download_name": download_name,
                }));
                result
            }
            Err(error) => BrowserReplResult::error(error),
        }
    }
}

fn repl_program_latex(
    program: &[crate::ast::Statement],
    symbols: &crate::lexer::SymbolTable,
) -> Option<String> {
    let renderer = LatexRenderer::new(symbols);
    match &program.last()?.kind {
        StatementKind::Expr(expression) | StatementKind::Print(expression) => {
            Some(renderer.expression(expression))
        }
        StatementKind::Let {
            name,
            value: Some(value),
        } => Some(format!(
            "{} = {}",
            renderer.identifier(symbols.resolve(*name)),
            renderer.expression(value)
        )),
        StatementKind::Function {
            name,
            params,
            body: FunctionBody::Expression(body),
        } => {
            let params = params
                .iter()
                .map(|param| renderer.identifier(symbols.resolve(*param)))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "{}\\left({params}\\right) = {}",
                renderer.identifier(symbols.resolve(*name)),
                renderer.expression(body)
            ))
        }
        _ => None,
    }
}

fn repl_source_latex(source: &str, symbols: &SymbolTable) -> Option<String> {
    let submission = if source.trim_end().ends_with(';') || source.trim_end().ends_with('}') {
        source.to_owned()
    } else {
        format!("{source};")
    };
    let mut scanner = Scanner::with_symbols(&submission, symbols.clone());
    let tokens = scanner.scan_tokens().clone();
    if !scanner.errors().is_empty() {
        return None;
    }
    let symbols = scanner.into_symbols();
    let program = Parser::new(tokens).parse().ok()?;
    repl_program_latex(&program, &symbols)
}

thread_local! {
    static REPL: RefCell<BrowserRepl> = RefCell::new(BrowserRepl::new());
}

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
    if parsed["kind"].as_str() == Some("file_import") {
        let name = parsed["name"].as_str().unwrap_or("");
        let content = parsed["content"].as_str().unwrap_or("");
        let result = crate::runtime_io::browser_io().insert_file(name, content.to_owned());
        return json!({"generation": generation, "kind": "file_import", "error": result.err().map(|error| error.to_string())}).to_string();
    }
    if let Some(inputs) = parsed["inputs"].as_array() {
        let io = crate::runtime_io::browser_io();
        io.clear_inputs();
        for input in inputs.iter().filter_map(Value::as_str) {
            io.queue_input(input.to_owned());
        }
    }
    let source = parsed["source"].as_str().unwrap_or("");
    if source.chars().count() > browser_limits().max_source_chars {
        return json!({ "generation": generation, "error": "Source exceeds the browser size limit" }).to_string();
    }
    match parsed["kind"].as_str().unwrap_or("run") {
        "repl" | "repl_numeric" => {
            let input = parsed["input"].as_str().unwrap_or("");
            let result = REPL.with(|repl| {
                repl.borrow_mut()
                    .submit(input, parsed["kind"].as_str() == Some("repl_numeric"))
            });
            let input_required = result
                .error
                .as_ref()
                .and_then(|message| message.split("input required: ").nth(1))
                .map(|rest| rest.lines().next().unwrap_or("").to_owned());
            json!({
                "generation": generation,
                "kind": "repl",
                "input": input,
                "output": result.output,
                "printed": result.printed,
                "error": result.error,
                "input_latex": result.input_latex,
                "result_latex": result.result_latex,
                "plot": result.plot,
                "clear": result.clear,
                "numeric": parsed["kind"].as_str() == Some("repl_numeric"),
                "input_required": input_required,
            })
            .to_string()
        }
        "plot" => {
            let view = parsed["view"].as_array().and_then(|values| {
                if values.len() != 4 {
                    return None;
                }
                Some([
                    values[0].as_f64()?,
                    values[1].as_f64()?,
                    values[2].as_f64()?,
                    values[3].as_f64()?,
                ])
            });
            plot_request(
                source,
                parsed["expression"].as_str().unwrap_or(""),
                generation,
                view,
                parsed["fit_y"].as_bool().unwrap_or(false),
            )
        }
        _ => run_request(source, generation, parsed["inputs"].as_array()),
    }
}

fn run_request(source: &str, generation: u64, inputs: Option<&Vec<Value>>) -> String {
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
    let execution_error = execution.err();
    let input_required = execution_error.as_ref().and_then(|error| {
        error
            .to_string()
            .strip_prefix("input required: ")
            .map(str::to_owned)
    });
    let runtime_error = execution_error.map(|error| error.render(source, "browser.scas"));
    // The document renderer evaluates independently and needs the same input answers.
    // It consumes its own queue so both output and preview see identical values.
    let io = crate::runtime_io::browser_io();
    io.clear_inputs();
    for input in inputs.into_iter().flatten().filter_map(Value::as_str) {
        io.queue_input(input.to_owned());
    }
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
                "input_required": input_required,
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
            "input_required": input_required,
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

fn plot_request(
    source: &str,
    expression: &str,
    generation: u64,
    view: Option<[f64; 4]>,
    fit_y: bool,
) -> String {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    if !scanner.errors().is_empty() {
        return json!({ "generation": generation, "kind": "plot", "error": "Fix source errors before plotting" })
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
                return json!({ "generation": generation, "kind": "plot", "error": error.message })
                    .to_string();
            }
        }
    }
    let range = view.and_then(|[x_min, x_max, _, _]| {
        (x_min.is_finite()
            && x_max.is_finite()
            && x_min < x_max
            && (1e-6..=1e9).contains(&(x_max - x_min)))
        .then_some((x_min, x_max))
    });
    if view.is_some() && range.is_none() {
        return json!({ "generation": generation, "kind": "plot", "error": "Graph view is outside supported bounds" }).to_string();
    }
    match plot::execute_plot_command_in_memory(
        &mut interpreter,
        &mut symbols,
        expression,
        64,
        17,
        range,
    ) {
        Ok((outcome, redirect)) => {
            if redirect.is_some() {
                return json!({ "generation": generation, "kind": "plot", "error": "Use Download SVG in Graph view to save a browser plot" }).to_string();
            }
            let svg_bounds = if let Some([x_min, x_max, y_min, y_max]) = view {
                if fit_y {
                    outcome.bounds
                } else if y_min.is_finite() && y_max.is_finite() && y_min < y_max {
                    plot::PlotBounds::new(x_min, x_max, y_min, y_max)
                } else {
                    outcome.bounds
                }
            } else {
                outcome.bounds
            };
            let svg = plot::render_svg(&outcome.series, &svg_bounds, 800, 520);
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
                "kind": "plot",
                "series": series,
                "bounds": [outcome.bounds.x_min, outcome.bounds.x_max, outcome.bounds.y_min, outcome.bounds.y_max],
                "svg": svg,
            }).to_string()
        }
        Err(error) => {
            json!({ "generation": generation, "kind": "plot", "error": error }).to_string()
        }
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
    fn browser_starter_renders_a_document_fragment() {
        let response: Value = serde_json::from_str(&process_request(
            &json!({
                "generation": 1, "source": STARTER,
            })
            .to_string(),
        ))
        .unwrap();
        assert!(response["error"].is_null(), "{response}");
        assert!(
            !response["fragments"].as_array().unwrap().is_empty(),
            "{response}"
        );
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
    fn browser_plot_resamples_the_requested_view() {
        let response: Value = serde_json::from_str(&process_request(
            &json!({
                "generation": 4,
                "kind": "plot",
                "source": "",
                "expression": "x^2",
                "view": [2.0, 4.0, -1.0, 10.0],
            })
            .to_string(),
        ))
        .unwrap();
        let samples = response["series"][0]["samples"].as_array().unwrap();
        assert_eq!(samples.first().unwrap()[0], 2.0);
        assert_eq!(samples.last().unwrap()[0], 4.0);
        assert!(response["svg"].as_str().unwrap().contains("<svg"));
        assert!(response["error"].is_null());
    }

    #[test]
    fn browser_repl_persists_definitions_and_recovers_after_errors() {
        let request = |input: &str| -> Value {
            serde_json::from_str(&process_request(
                &json!({
                    "generation": 1, "kind": "repl", "input": input
                })
                .to_string(),
            ))
            .unwrap()
        };
        request(":reset");
        assert!(request("let offset = 7;")["error"].is_null());
        assert_eq!(request("offset + 5")["output"], "12");
        assert!(!request("let = ;")["error"].is_null());
        assert_eq!(request("offset")["output"], "7");
        assert!(request(":definitions")["output"]
            .as_str()
            .unwrap()
            .contains("offset"));
        assert_eq!(request(":reset")["output"], "Session reset");
        assert_eq!(request("offset")["output"], "offset");
    }

    #[test]
    fn browser_repl_matches_native_math_numeric_and_command_metadata() {
        let request = |kind: &str, input: &str| -> Value {
            serde_json::from_str(&process_request(
                &json!({
                    "generation": 12, "kind": kind, "input": input
                })
                .to_string(),
            ))
            .unwrap()
        };

        request("repl", ":reset");
        let exact = request("repl", "sqrt(2)");
        assert_eq!(exact["kind"], "repl");
        assert_eq!(exact["numeric"], false);
        assert!(exact["error"].is_null(), "{exact}");
        assert!(exact["input_latex"].as_str().unwrap().contains("sqrt"));
        assert!(!exact["result_latex"].as_str().unwrap().is_empty());

        let precision = request("repl", ":precision 12");
        assert_eq!(precision["output"], "decimal precision set to 12 digits");
        let numerical = request("repl_numeric", "sqrt(2)");
        assert_eq!(numerical["numeric"], true);
        assert!(numerical["error"].is_null(), "{numerical}");
        assert!(numerical["output"].as_str().unwrap().starts_with("1.414"));
        assert!(!numerical["result_latex"].as_str().unwrap().is_empty());
        assert!(!numerical["input_latex"].as_str().unwrap().contains("evalf"));

        let clear = request("repl", ":clear");
        assert_eq!(clear["clear"], true);
    }

    #[test]
    fn browser_repl_plots_session_definitions_and_recovers_from_bad_commands() {
        let request = |input: &str| -> Value {
            serde_json::from_str(&process_request(
                &json!({ "generation": 13, "kind": "repl", "input": input }).to_string(),
            ))
            .unwrap()
        };
        request(":reset");
        assert!(request("f(x) = x^2 - 2")["error"].is_null());
        let plot = request(":plot f(x), sin(x) from -3 to 3");
        assert!(plot["error"].is_null(), "{plot}");
        assert_eq!(plot["plot"]["series"].as_array().unwrap().len(), 2);
        assert_eq!(plot["plot"]["bounds"][0], -3.0);
        assert_eq!(plot["plot"]["bounds"][1], 3.0);
        assert!(plot["plot"]["ascii"].as_str().unwrap().contains('*'));
        assert!(plot["plot"]["svg"].as_str().unwrap().contains("<svg"));
        assert_eq!(plot["plot"]["vector"], false);

        let vector = request(":plot(svg sin(x))");
        assert!(vector["error"].is_null(), "{vector}");
        assert_eq!(vector["plot"]["vector"], true);
        let export = request(":plot svg f(x) > graph.svg");
        assert!(export["error"].is_null(), "{export}");
        assert_eq!(export["plot"]["download_name"], "graph.svg");
        assert_eq!(export["plot"]["vector"], true);
        assert!(request(":plot")["output"]
            .as_str()
            .unwrap()
            .contains("usage:"));
        assert!(request(":help")["output"]
            .as_str()
            .unwrap()
            .contains(":plot EXPR"));
        assert!(!request(":plot f(x) from 3 to -3")["error"].is_null());
        assert_eq!(request("f(3)")["output"], "7");
        assert!(request(":history")["output"]
            .as_str()
            .unwrap()
            .contains(":plot f(x)"));
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
