use scas::ast::StatementKind;
use scas::interpreter::Interpreter;
use scas::latex::LatexRenderer;
use scas::lexer::{Scanner, SymbolTable};
use scas::parser::Parser;
use scas::{execute, execute_with_options, ExecutionOptions, Limits};
use std::hint::black_box;
use std::num::NonZeroU64;
use std::time::{Duration, Instant};

fn measure(iterations: u32, mut operation: impl FnMut()) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        operation();
    }
    start.elapsed() / iterations
}

fn report(name: &str, duration: Duration) {
    println!(
        "{name:24} {:>12.3} us/iteration",
        duration.as_secs_f64() * 1e6
    );
}

fn main() {
    let repeated = "let value = 12345678901234567890;\n".repeat(1_000);
    report(
        "scan 1k declarations",
        measure(200, || {
            let mut scanner = Scanner::new(black_box(&repeated));
            black_box(scanner.scan_tokens());
        }),
    );

    report(
        "parse 1k declarations",
        measure(100, || {
            let mut scanner = Scanner::new(black_box(&repeated));
            let tokens = scanner.scan_tokens().clone();
            black_box(Parser::new(tokens).parse().unwrap());
        }),
    );

    let arithmetic = "print((123456789012345678901234567890^20 + 1) / 7);";
    report(
        "large exact arithmetic",
        measure(500, || {
            execute(black_box(arithmetic), Vec::new()).unwrap();
        }),
    );

    let symbolic = "print(expand((x + y + z)^8));";
    report(
        "symbolic expansion",
        measure(100, || {
            execute(black_box(symbolic), Vec::new()).unwrap();
        }),
    );

    let calls = "fn f(x) = x + 1; let n = 0; for let i = 0; i < 1000; i += 1 { n = f(n); }";
    report(
        "1k function calls",
        measure(100, || {
            execute(black_box(calls), Vec::new()).unwrap();
        }),
    );

    let tour = include_str!("../tests/should_pass/symbolic_algebra.scas");
    report(
        "algebra tour end-to-end",
        measure(100, || {
            execute(black_box(tour), Vec::new()).unwrap();
        }),
    );

    let definite_integrals = concat!(
        "print(definite_integral(x*exp(x), x, -1, 1));",
        "print(definite_integral(x + x^2, x, -1, 1));",
        "print(definite_integral(x^2, x, -2, 2));",
    );
    report(
        "3 definite integrals",
        measure(100, || {
            execute(black_box(definite_integrals), Vec::new()).unwrap();
        }),
    );

    report(
        "gamma (50 digits)",
        measure(20, || {
            execute_with_options(
                black_box("print(gamma(0.5));"),
                Vec::new(),
                ExecutionOptions {
                    limits: Limits::default(),
                    decimal_precision: NonZeroU64::new(50).unwrap(),
                },
            )
            .unwrap();
        }),
    );

    let plain_render_lines = "\\[ sin(pi / 6) \\]\n".repeat(100);
    report(
        "document 100 plain",
        measure(100, || {
            black_box(scas::document::build(black_box(&plain_render_lines)).unwrap());
        }),
    );
    let evaluated_render_lines = "\\[ sin(pi / 6) => \\]\n".repeat(100);
    report(
        "document 100 results",
        measure(100, || {
            black_box(scas::document::build(black_box(&evaluated_render_lines)).unwrap());
        }),
    );
    let matrix_plain = "\\[ rref([1,2,3,4;5,6,7,8;2,3,5,7;11,13,17,19]) \\]\n".repeat(25);
    report(
        "document 25 rref",
        measure(20, || {
            black_box(scas::document::build(black_box(&matrix_plain)).unwrap());
        }),
    );
    let matrix_results = "\\[ rref([1,2,3,4;5,6,7,8;2,3,5,7;11,13,17,19]) => \\]\n".repeat(25);
    report(
        "document 25 rref steps",
        measure(20, || {
            black_box(scas::document::build(black_box(&matrix_results)).unwrap());
        }),
    );
    report(
        "live 25 rref results",
        measure(20, || {
            black_box(
                scas::document::build_live_with_limits(
                    black_box(&matrix_results),
                    Limits::default(),
                )
                .unwrap(),
            );
        }),
    );
    let gamma_results = "\\[ evalf(gamma(0.5)) => \\]\n".repeat(25);
    report(
        "document 25 gamma",
        measure(5, || {
            black_box(scas::document::build(black_box(&gamma_results)).unwrap());
        }),
    );
    report(
        "live 25 gamma",
        measure(5, || {
            black_box(
                scas::document::build_live_with_limits(
                    black_box(&gamma_results),
                    Limits::default(),
                )
                .unwrap(),
            );
        }),
    );

    let mut latex_scanner =
        Scanner::new("print(integral(sin(2alpha_1) / (1 + x^2), x) + [1, 2; 3, 4]);");
    let latex_tokens = latex_scanner.scan_tokens().clone();
    let latex_symbols = latex_scanner.into_symbols();
    let latex_program = Parser::new(latex_tokens).parse().unwrap();
    let StatementKind::Print(latex_expression) = &latex_program[0].kind else {
        unreachable!("benchmark source is a print statement");
    };
    let latex_renderer = LatexRenderer::new(&latex_symbols);
    let mut latex_output = String::new();
    report(
        "LaTeX expression render",
        measure(10_000, || {
            latex_renderer.expression_into(black_box(latex_expression), &mut latex_output);
            black_box(&latex_output);
        }),
    );

    // Plot sampling feeds the interactive graphers. The first case compiles
    // to the f64 fast path; the second (a user function) must fall back to
    // the exact evaluator, which is the slow lane of the same pipeline.
    let mut plot_symbols = SymbolTable::new();
    let mut plot_interpreter = Interpreter::new(plot_symbols.clone(), Vec::new());
    report(
        "plot sample 1k (fast)",
        measure(50, || {
            black_box(
                scas::plot::sample_source(
                    &mut plot_interpreter,
                    &mut plot_symbols,
                    black_box("sin(40*x) / (1 + x^2)"),
                    -10.0,
                    10.0,
                    1_000,
                )
                .unwrap(),
            );
        }),
    );
    report(
        "plot sample 1k (exact)",
        measure(10, || {
            black_box(
                scas::plot::sample_source(
                    &mut plot_interpreter,
                    &mut plot_symbols,
                    black_box("g(t) = t^2 + sin(t)"),
                    -10.0,
                    10.0,
                    1_000,
                )
                .unwrap(),
            );
        }),
    );

    // Keep direct construction covered so benchmark builds catch public API drift.
    let mut scanner = Scanner::new("print(1);");
    let tokens = scanner.scan_tokens().clone();
    let symbols = scanner.into_symbols();
    let program = Parser::new(tokens).parse().unwrap();
    let mut interpreter = Interpreter::new(symbols, Vec::new());
    interpreter.run(&program).unwrap();
}
