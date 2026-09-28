use scas::ast::StatementKind;
use scas::execute;
use scas::interpreter::Interpreter;
use scas::latex::LatexRenderer;
use scas::lexer::Scanner;
use scas::parser::Parser;
use std::hint::black_box;
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

    // Keep direct construction covered so benchmark builds catch public API drift.
    let mut scanner = Scanner::new("print(1);");
    let tokens = scanner.scan_tokens().clone();
    let symbols = scanner.into_symbols();
    let program = Parser::new(tokens).parse().unwrap();
    let mut interpreter = Interpreter::new(symbols, Vec::new());
    interpreter.run(&program).unwrap();
}
