//! Front-end utilities shared by the self-documenter and future IDE clients.

use crate::ast::statement::FunctionBody;
use crate::ast::{AssignOp, Expr, ExprKind, SourceSpan, Statement, StatementKind};
use crate::interpreter::Interpreter;
use crate::latex::LatexRenderer;
use crate::lexer::Scanner;
use crate::parser::Parser;
use crate::plot::{self, PlotBounds};
use crate::{ExecutionError, Limits, RuntimeError};
use std::io;
use std::num::NonZeroU64;
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
            "\\documentclass[11pt]{{article}}\n\\usepackage[T1]{{fontenc}}\n\\usepackage{{lmodern}}\n\\usepackage{{microtype}}\n\\usepackage{{amsmath}}\n\\usepackage{{amssymb}}\n\\usepackage{{upgreek}}\n\\usepackage{{booktabs}}\n\\usepackage{{xcolor}}\n\\usepackage[margin=1in]{{geometry}}\n\\setlength{{\\parindent}}{{0pt}}\n\\setlength{{\\parskip}}{{0.65em}}\n\\setlength{{\\jot}}{{0.45em}}\n\\emergencystretch=2em\n\\allowdisplaybreaks[4]\n{user_packages}\\begin{{document}}\n{}\n\\end{{document}}\n",
            self.body()
        )
    }
}

/// Parse, execute, and assemble one self-documenting SCAS source file.
pub fn build(source: &str) -> Result<Document, ExecutionError> {
    build_with_options_and_limits(source, BuildOptions::default(), Limits::default(), false)
}

pub fn build_with_options(source: &str, options: BuildOptions) -> Result<Document, ExecutionError> {
    build_with_options_and_limits(source, options, Limits::default(), false)
}

/// Build a live document with a caller-selected evaluation budget. Editors
/// use this to keep expensive or accidental computations from monopolizing a
/// machine; explicit runs and exports retain the normal full budget.
pub fn build_with_limits(source: &str, limits: Limits) -> Result<Document, ExecutionError> {
    build_with_options_and_limits(source, BuildOptions::default(), limits, false)
}

/// Build a latency-sensitive live preview. The caller-selected limits keep
/// evaluation bounded, while dynamic result markers retain the same detailed
/// derivations as explicit builds.
pub fn build_live_with_limits(source: &str, limits: Limits) -> Result<Document, ExecutionError> {
    build_with_options_and_limits(source, BuildOptions::default(), limits, true)
}

fn build_with_options_and_limits(
    source: &str,
    options: BuildOptions,
    limits: Limits,
    latency_sensitive: bool,
) -> Result<Document, ExecutionError> {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    if !scanner.errors().is_empty() {
        return Err(ExecutionError::Scan(scanner.errors().to_vec()));
    }
    let symbols = scanner.into_symbols();
    let statements = Parser::new(tokens).parse().map_err(ExecutionError::Parse)?;
    let mut interpreter = Interpreter::with_limits(symbols, io::sink(), limits);
    if latency_sensitive {
        let precision = limits.max_decimal_precision.min(34).max(1);
        interpreter
            .set_decimal_precision(NonZeroU64::new(precision).expect("precision is at least one"))
            .map_err(ExecutionError::Runtime)?;
    }
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
        StatementKind::Expr(expression) => {
            if let Some(plot) = render_plot_call(expression, interpreter) {
                let latex = plot.unwrap_or_else(|message| {
                    document.diagnostics.push(DocumentDiagnostic {
                        span: statement.span,
                        severity: DiagnosticSeverity::Error,
                        message,
                    });
                    r"\text{\textbf{plot unavailable}}".to_owned()
                });
                document.fragments.push(DocumentFragment {
                    span: statement.span,
                    kind: FragmentKind::DisplayMath,
                    latex: format!(r"\[{}\]", latex),
                });
                return Ok(());
            }
            interpreter
                .execute_statement(statement)
                .map_err(ExecutionError::Runtime)?;
        }
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
            if let Some(plot) = render_plot_call(expression, interpreter) {
                let latex = plot.unwrap_or_else(|message| {
                    document.diagnostics.push(DocumentDiagnostic {
                        span: statement.span,
                        severity: DiagnosticSeverity::Error,
                        message,
                    });
                    r"\text{\textbf{plot unavailable}}".to_owned()
                });
                document.fragments.push(DocumentFragment {
                    span: statement.span,
                    kind: FragmentKind::DisplayMath,
                    latex: format!(r"\[{}\]", latex),
                });
                return Ok(());
            }
            let source = LatexRenderer::new(interpreter.symbols()).expression(expression);
            let latex = if *show_result {
                let result = interpreter.evaluate_documented_expression(expression);
                match result {
                    Ok(value) => format!(
                        r"\[{}\]",
                        render_evaluation(expression, &source, &value, interpreter)
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
        let row = match &statement.kind {
            StatementKind::DocComment(prose) => Some(format!(
                r"\intertext{{{}}}",
                interpolate(prose, statement.span, interpreter, document)
            )),
            StatementKind::Expr(expression) => {
                if let Some(plot) = render_plot_call(expression, interpreter) {
                    Some(format!(
                        "& {}",
                        plot.unwrap_or_else(|message| {
                            document.diagnostics.push(DocumentDiagnostic {
                                span: statement.span,
                                severity: DiagnosticSeverity::Error,
                                message,
                            });
                            r"\text{\textbf{plot unavailable}}".to_owned()
                        })
                    ))
                } else {
                    let text = LatexRenderer::new(interpreter.symbols()).expression(expression);
                    interpreter
                        .execute_statement(statement)
                        .map_err(ExecutionError::Runtime)?;
                    Some(text)
                }
            }
            StatementKind::Show(expression) => {
                if let Some(plot) = render_plot_call(expression, interpreter) {
                    Some(format!(
                        "& {}",
                        plot.unwrap_or_else(|message| {
                            document.diagnostics.push(DocumentDiagnostic {
                                span: statement.span,
                                severity: DiagnosticSeverity::Error,
                                message,
                            });
                            r"\text{\textbf{plot unavailable}}".to_owned()
                        })
                    ))
                } else {
                    let text = LatexRenderer::new(interpreter.symbols()).expression(expression);
                    let result = interpreter.evaluate_documented_expression(expression);
                    Some(match result {
                        Ok(value) => render_evaluation(expression, &text, &value, interpreter),
                        Err(error) => {
                            push_runtime_diagnostic(error, statement.span, document);
                            format!(r"{text} &= \text{{\textbf{{?}}}}")
                        }
                    })
                }
            }
            StatementKind::Let { name, value } => {
                let row = value.as_ref().map(|value| {
                    let renderer = LatexRenderer::new(interpreter.symbols());
                    let name = renderer.identifier(interpreter.symbols().resolve(*name));
                    format!("{} &= {}", name, renderer.expression(value))
                });
                interpreter
                    .execute_statement(statement)
                    .map_err(ExecutionError::Runtime)?;
                row
            }
            StatementKind::Assign { name, op, value } => {
                let renderer = LatexRenderer::new(interpreter.symbols());
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
                let renderer = LatexRenderer::new(interpreter.symbols());
                let name = renderer.identifier(interpreter.symbols().resolve(*name));
                let params = params
                    .iter()
                    .map(|symbol| renderer.identifier(interpreter.symbols().resolve(*symbol)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let body = renderer.expression(body);
                interpreter
                    .execute_statement(statement)
                    .map_err(ExecutionError::Runtime)?;
                Some(format!(r"{name}\left({params}\right) &= {body}"))
            }
            StatementKind::Chain(statements) => {
                let mut parts = Vec::new();
                for chained in statements {
                    let renderer = LatexRenderer::new(interpreter.symbols());
                    match &chained.kind {
                        StatementKind::Let {
                            name,
                            value: Some(value),
                        } => parts.push(format!(
                            "{} = {}",
                            renderer.identifier(interpreter.symbols().resolve(*name)),
                            renderer.expression(value)
                        )),
                        StatementKind::Let { value: None, .. } => {}
                        StatementKind::Assign { name, value, .. } => parts.push(format!(
                            "{} = {}",
                            renderer.identifier(interpreter.symbols().resolve(*name)),
                            renderer.expression(value)
                        )),
                        StatementKind::Expr(expression) => {
                            parts.push(renderer.expression(expression))
                        }
                        _ => {}
                    }
                    interpreter
                        .execute_statement(chained)
                        .map_err(ExecutionError::Runtime)?;
                }
                (!parts.is_empty()).then(|| parts.join(r" \quad "))
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
            latex: format!(r"\begin{{align*}}{}\end{{align*}}", rows.join(" \\\\\n")),
        });
    }
    Ok(())
}

/// Evaluate `plot(expression[, variable], lower, upper)` in document scope
/// and return a vector-LaTeX picture suitable for the normal LaTeX renderer.
fn render_plot_call(
    expression: &Expr,
    interpreter: &mut Interpreter<io::Sink>,
) -> Option<Result<String, String>> {
    let ExprKind::Call { callee, args } = &expression.kind else {
        return None;
    };
    let ExprKind::Variable(function) = callee.kind else {
        return None;
    };
    if interpreter.symbols().resolve(function) != "plot" {
        return None;
    }

    Some((|| {
        if args.len() != 3 && args.len() != 4 {
            return Err(
                "plot expects plot(expression, lower, upper) or plot(expression, variable, lower, upper)"
                    .to_owned(),
            );
        }
        let variable = if args.len() == 4 {
            let ExprKind::Variable(variable) = &args[1].kind else {
                return Err("plot's independent variable must be a name".to_owned());
            };
            interpreter.symbols().resolve(*variable).to_owned()
        } else {
            "x".to_owned()
        };
        let lower_index = if args.len() == 4 { 2 } else { 1 };
        let upper_index = lower_index + 1;
        let mut symbols = interpreter.symbols().clone();
        let lower =
            plot::evaluate_expression_scalar(interpreter, &mut symbols, &args[lower_index])?;
        let upper =
            plot::evaluate_expression_scalar(interpreter, &mut symbols, &args[upper_index])?;
        if lower >= upper {
            return Err("plot bounds must satisfy lower < upper".to_owned());
        }

        let expression_latex = LatexRenderer::new(interpreter.symbols()).expression(&args[0]);
        let series = plot::sample_expression(
            interpreter,
            &mut symbols,
            &args[0],
            expression_latex,
            &variable,
            lower,
            upper,
            320,
        )?;
        let bounds = plot::auto_y_bounds(std::slice::from_ref(&series), lower, upper);
        Ok(plot::render_latex_picture(
            &[series],
            &PlotBounds::new(lower, upper, bounds.y_min, bounds.y_max),
            400.0,
            220.0,
        ))
    })())
}

/// Give result markers mathematical meaning instead of blindly appending an
/// equals sign to every call. Transformations use an implication, while solve
/// expands its result into the equation(s) for the requested variable.
fn render_evaluation(
    expression: &Expr,
    source: &str,
    value: &crate::interpreter::value::Value,
    interpreter: &Interpreter<io::Sink>,
) -> String {
    use crate::ast::ExprKind;

    let renderer = LatexRenderer::new(interpreter.symbols());
    let result = renderer.value(value);
    let ExprKind::Call { callee, args } = &expression.kind else {
        return format!("{source} = {result}");
    };
    let ExprKind::Variable(function) = callee.kind else {
        return format!("{source} = {result}");
    };
    let name = interpreter.symbols().resolve(function);

    if name == "solve" && args.len() == 2 {
        let equation = renderer.expression(&args[0]);
        let variable = renderer.expression(&args[1]);
        if let crate::interpreter::value::Value::Array(roots) = value {
            let solutions = if roots.is_empty() {
                r"\text{no solutions}".to_owned()
            } else {
                roots
                    .iter()
                    .map(|root| format!("{variable} = {}", renderer.value(root)))
                    .collect::<Vec<_>>()
                    .join(r",\quad ")
            };
            return format!(
                r"\begin{{aligned}}{equation} &= 0 \\ &\implies {solutions}\end{{aligned}}"
            );
        }
    }

    if name == "derivative" && args.len() == 2 {
        let function = renderer.expression(&args[0]);
        let variable = renderer.expression(&args[1]);
        return format!(
            r"\begin{{aligned}}f\left({variable}\right)&={function} \\ \frac{{df}}{{d{variable}}}&={result}\end{{aligned}}"
        );
    }
    if name == "integral" && args.len() == 2 {
        let integrand = renderer.expression(&args[0]);
        let variable = renderer.expression(&args[1]);
        return format!(
            r"\begin{{aligned}}{source}&={result}+C \\ \frac{{d}}{{d{variable}}}\left({result}\right)&={integrand}\end{{aligned}}"
        );
    }

    if let crate::interpreter::value::Value::Derivation { result, steps } = value {
        return render_derivation(name, args, source, result, steps, &renderer);
    }

    if matches!(
        name,
        "simplify"
            | "expand"
            | "factor"
            | "apart"
            | "horner"
            | "collect"
            | "subs"
            | "rref"
            | "inverse"
            | "transpose"
            | "solve_linear"
            | "gradient"
            | "jacobian"
            | "hessian"
            | "definite_integral"
    ) {
        return format!(r"{source} \implies {result}");
    }
    format!("{source} = {result}")
}

fn render_derivation(
    name: &str,
    args: &[Expr],
    source: &str,
    result: &crate::interpreter::value::Value,
    steps: &[crate::interpreter::value::DerivationStep],
    renderer: &LatexRenderer<'_>,
) -> String {
    use crate::interpreter::value::StepOperation;

    let base_name = name.strip_suffix("_steps").unwrap_or(name);
    if base_name == "trace" {
        if let Some(terms) = steps.iter().find_map(|step| match &step.value {
            crate::interpreter::value::Value::Array(values)
                if values
                    .iter()
                    .all(|value| !matches!(value, crate::interpreter::value::Value::Array(_))) =>
            {
                Some(
                    values
                        .iter()
                        .map(|value| renderer.value(value))
                        .collect::<Vec<_>>()
                        .join(" + "),
                )
            }
            _ => None,
        }) {
            return format!(
                r"\begin{{aligned}}{source} &= {terms} \\ &= {}\end{{aligned}}",
                renderer.value(result)
            );
        }
    }
    let input = match (base_name, args) {
        ("inverse", [matrix]) => format!(r"\left[{}\mid I\right]", renderer.expression(matrix)),
        ("solve_linear", [matrix, vector]) => format!(
            r"\left[{}\mid {}\right]",
            renderer.expression(matrix),
            renderer.expression(vector)
        ),
        (_, [first, ..]) => renderer.expression(first),
        _ => source.to_owned(),
    };
    let mut rows = Vec::new();
    let mut last_rendered_value = None;
    for step in steps {
        let value = renderer.value(&step.value);
        match &step.operation {
            StepOperation::Start => {
                rows.push(format!(r"{input} &= {value}"));
                last_rendered_value = Some(&step.value);
            }
            StepOperation::EvaluateArguments { values } => {
                if base_name == "matmul" && values.len() == 2 {
                    rows.push(format!(
                        r"{source} &= {}\,{}",
                        renderer.value(&values[0]),
                        renderer.value(&values[1])
                    ));
                }
            }
            StepOperation::SwapRows { first, second } => {
                rows.push(format!(
                    r"&\sim {value} && \left(R_{{{}}}\leftrightarrow R_{{{}}}\right)",
                    first + 1,
                    second + 1
                ));
                last_rendered_value = Some(&step.value);
            }
            StepOperation::ScaleRow { row, divisor } => {
                rows.push(format!(
                    r"&\sim {value} && \left(R_{{{}}}\leftarrow \frac{{1}}{{{}}}R_{{{}}}\right)",
                    row + 1,
                    renderer.value(divisor),
                    row + 1
                ));
                last_rendered_value = Some(&step.value);
            }
            StepOperation::AddRowMultiple {
                target,
                source,
                factor,
            } => {
                rows.push(format!(
                    r"&\sim {value} && \left(R_{{{}}}\leftarrow R_{{{}}}-\left({}\right)R_{{{}}}\right)",
                    target + 1,
                    target + 1,
                    renderer.value(factor),
                    source + 1
                ));
                last_rendered_value = Some(&step.value);
            }
            StepOperation::Explanation(explanation) => {
                let left = if rows.is_empty() {
                    source.to_owned()
                } else if matches!(base_name, "det" | "rank") {
                    semantic_result_name(base_name, args, renderer)
                } else {
                    String::new()
                };
                if base_name == "matmul" {
                    rows.push(format!(r"&= {value}"));
                } else if base_name == "characteristic_polynomial" {
                    rows.push(format!(r"{left} &= {value}"));
                } else {
                    rows.push(format!(r"{left} &= {value} && \left({explanation}\right)"));
                }
                last_rendered_value = Some(&step.value);
            }
        }
    }
    let needs_named_result = matches!(base_name, "inverse" | "rref" | "solve_linear");
    let result_already_shown = last_rendered_value.is_some_and(|value| value.equals(result));
    if needs_named_result || !result_already_shown {
        rows.push(format!(
            r"{} &= {}",
            semantic_result_name(base_name, args, renderer),
            renderer.value(result)
        ));
    }
    format!(r"\begin{{aligned}}{}\end{{aligned}}", rows.join(" \\\\\n"))
}

fn semantic_result_name(name: &str, args: &[Expr], renderer: &LatexRenderer<'_>) -> String {
    match (name, args) {
        ("rref", [matrix]) => format!(
            r"\operatorname{{rref}}\left({}\right)",
            renderer.expression(matrix)
        ),
        ("rank", [matrix]) => format!(
            r"\operatorname{{rank}}\left({}\right)",
            renderer.expression(matrix)
        ),
        ("inverse", [matrix]) => format!(r"{}^{{-1}}", renderer.expression(matrix)),
        ("solve_linear", [matrix, vector]) => format!(
            r"\operatorname{{solve}}\left({},{}\right)",
            renderer.expression(matrix),
            renderer.expression(vector)
        ),
        ("matmul", _)
        | ("trace", _)
        | ("det", _)
        | ("characteristic_polynomial", _)
        | ("eigenvalues", _)
        | ("eigenvectors", _)
        | ("transpose", _) => {
            let rendered = args
                .iter()
                .map(|argument| renderer.expression(argument))
                .collect::<Vec<_>>();
            render_conventional_call(name, &rendered)
        }
        _ => {
            let name = renderer.identifier(name);
            let rendered = args
                .iter()
                .map(|arg| renderer.expression(arg))
                .collect::<Vec<_>>()
                .join(", ");
            format!(r"\operatorname{{{name}}}\left({rendered}\right)")
        }
    }
}

fn render_conventional_call(name: &str, args: &[String]) -> String {
    match (name, args) {
        ("matmul", [left, right]) => format!(r"{left}\,{right}"),
        ("trace", [matrix]) => format!(r"\operatorname{{tr}}\left({matrix}\right)"),
        ("det", [matrix]) => format!(r"\det\left({matrix}\right)"),
        ("transpose", [matrix]) => format!(r"{matrix}^{{\mathsf T}}"),
        ("characteristic_polynomial", [matrix, variable]) => {
            format!(r"p_{{{matrix}}}\left({variable}\right)")
        }
        ("eigenvalues", [matrix]) => format!(r"\sigma\left({matrix}\right)"),
        ("eigenvectors", [matrix]) => format!(r"\mathcal{{E}}\left({matrix}\right)"),
        _ => format!(r"\operatorname{{{name}}}\left({}\right)", args.join(", ")),
    }
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
        assert!(document.fragments[1].latex.contains(r"x + 3 = 5"));
        assert!(document.fragments[2].latex.contains(r"y &= x \cdot 4"));
        assert!(document.fragments[2].latex.contains(r"\frac{y}{2} = 4"));
        assert!(document.latex().contains(r"\usepackage{amsmath}"));
        assert!(document.latex().contains(r"\allowdisplaybreaks[4]"));
        assert!(document
            .fragments
            .iter()
            .all(|fragment| fragment.span.line > 0));
    }

    #[test]
    fn user_function_definitions_and_calls_use_math_identifiers() {
        let document = build("\\{ f(x)=2x+1; solve(f(x),x) =>; \\}\n").unwrap();
        assert!(document.body().contains(r"f\left(x\right) &= 2x + 1"));
        assert!(document.body().contains(r"f\left(x\right) &= 0"));
        assert!(!document.body().contains(r"\operatorname{f}"));
    }

    #[test]
    fn plot_calls_render_vector_latex_in_display_math_and_blocks() {
        let source = r#"
            let x = 99;
            plot(sin(x), x, -pi, pi);
            \[
                plot(x^2, x, -2, 2)
            \]
            \{
                plot(cos(t), t, -pi, pi) =>;
            \}
            \[ x => \]
            print(x);
        "#;
        let document = build(source).unwrap();
        assert_eq!(document.fragments.len(), 4);
        assert_eq!(document.fragments[0].kind, FragmentKind::DisplayMath);
        assert!(document.fragments[0].latex.contains(r"\begin{picture}"));
        assert!(document.fragments[0].latex.contains(r"\qbezier"));
        assert!(document.fragments[0].latex.contains(r"\color{blue}"));
        assert!(document.fragments[0].latex.contains(r"\circle*"));
        assert!(document.fragments[1].latex.contains(r"\begin{picture}"));
        assert!(document.fragments[2].latex.contains(r"\begin{picture}"));
        assert!(document.fragments[2].latex.contains("& "));
        assert!(document.fragments[3].latex.contains("x = 99"));
        assert!(document.latex().contains(r"\usepackage{xcolor}"));
        assert!(document.diagnostics.is_empty());
        assert!(!document.body().contains("plot unavailable"));
    }

    #[test]
    fn plot_supports_named_user_functions_and_rejects_bad_bounds() {
        let document = build(
            r#"
                f(t) = 2t + 1;
                \[ plot(f(t), t, -1, 1) \]
            "#,
        )
        .unwrap();
        assert!(document.body().contains(r"\begin{picture}"));
        assert!(document.body().contains(r"\qbezier"));

        let document = build(r#"\[ plot(x, x, 2, 1) \]"#).unwrap();
        assert_eq!(document.diagnostics.len(), 1);
        assert!(document.diagnostics[0]
            .message
            .contains("plot bounds must satisfy lower < upper"));
        assert!(document.body().contains("plot unavailable"));
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

    #[test]
    fn dynamic_results_chains_and_statement_rows_have_semantic_latex() {
        let document = build(
            r#"\{
solve(z^2-z, z) =>;
let long_name = 2, x = 1;
x = 3;
det_steps([1, 2; 3, 4]) =>;
\}
"#,
        )
        .unwrap();
        let latex = &document.fragments[0].latex;
        assert!(latex.contains(r"\mathrm{long}_{\mathrm{name}} = 2 \quad x = 1"));
        assert!(latex.contains(r"z^{2} - z &= 0"), "{latex}");
        assert!(latex.contains(r"&\implies z = 0"), "{latex}");
        assert!(latex.contains(r"z = 0"));
        assert!(latex.contains(r"z = 1"));
        assert!(latex.contains(r"\begin{aligned}"));
        assert!(latex.matches("\\\\\n").count() >= 3, "{latex}");
    }

    #[test]
    fn live_results_keep_full_determinant_and_rref_derivations() {
        let source = r#"
\[ det([1, 2; 3, 4]) => \]
\[ rref([1, 2; 3, 4]) => \]
"#;
        let detailed = build(source).unwrap();
        let live = build_live_with_limits(source, Limits::default()).unwrap();

        assert_eq!(live.body(), detailed.body());
        assert_eq!(live.body().matches(r"\sim").count(), 4, "{}", live.body());
        assert!(
            live.body()
                .contains(r"\text{product of pivots, adjusted for row swaps}"),
            "{}",
            live.body()
        );
        assert!(
            live.body()
                .contains(r"R_{2}\leftarrow R_{2}-\left(3\right)R_{1}"),
            "{}",
            live.body()
        );
        assert!(
            live.body().contains(r"R_{2}\leftarrow \frac{1}{-2}R_{2}"),
            "{}",
            live.body()
        );
    }

    #[test]
    fn row_reduction_documents_each_operation_and_equivalent_matrix() {
        let document = build(r#"\{ rref([1, 2; 3, 4]) =>; \}"#).unwrap();
        let latex = &document.fragments[0].latex;
        assert!(latex.contains(r"\sim"), "{latex}");
        assert!(
            latex.contains(r"R_{2}\leftarrow R_{2}-\left(3\right)R_{1}"),
            "{latex}"
        );
        assert!(
            latex.contains(r"R_{2}\leftarrow \frac{1}{-2}R_{2}"),
            "{latex}"
        );
        assert!(latex.contains(r"\operatorname{rref}"), "{latex}");
        assert!(
            latex.contains(r"\end{pmatrix} && \left(R_"),
            "row-operation annotations should sit beside their matrices: {latex}"
        );
    }
}
