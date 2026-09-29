# SCAS

SCAS is a dynamically typed interpreted language and computer
algebra system written in Rust. It supports arbitrary-size integers, exact
rationals, arbitrary-precision real and complex decimals, arrays, closures,
control flow, and numeric and bitwise operators.

## Requirements

- Rust 1.79 or newer with Cargo ([install Rust](https://rustup.rs/))

## Installation

Install from source with `cargo install --git https://github.com/starless333/scas`,
or download the versioned archive for Linux, macOS, or Windows from GitHub
Releases. Release archives include SHA-256 checksums and GitHub build-provenance
attestations. After installation, run `scas --version` to verify the executable.

## Running a program

Running SCAS in a terminal starts an interactive REPL. Definitions remain
available until the session ends, bare expressions print their values, and
multiline blocks receive a continuation prompt.

```text
$ cargo run --quiet
scas 0.1.0 — type :help for help, :quit to exit
> f(x) = x^2 + 2x + 1
> derivative(f(x), x)
2 * x + 2
> solve(f(x), x)
[-1]
```

Use `:help`, `:load`, `:definitions`, `:history`, `:reset`, `:clear`,
`:precision N`, or `:quit` to control the REPL.

When standard input is redirected, SCAS reads and executes a complete program.
From the repository root, run a `.scas` file with:

```sh
cargo run --quiet -- path/to/program.scas
```

For example, the included sample can be run with:

```sh
cargo run --quiet -- tests/should_pass/language_tour.scas
```

You can also pipe a short program directly into the interpreter:

```sh
printf 'print(2 ^ 8);\n' | cargo run --quiet
```

To build once and run the compiled executable:

```sh
cargo build --release
./target/release/scas path/to/program.scas
```

## Desktop GUI (early preview)

Build and launch the optional native editor with:

```sh
cargo run --features gui --bin scas-gui
```

The preview includes a `.scas` file tree, a syntax-highlighted editor,
background execution with an output panel, save support, and Greek input.
When `latex` and `dvipng` are installed, the live preview is typeset by LaTeX
itself in Computer Modern; the native structured renderer remains a fallback
when those commands are unavailable.
Gruber Darker is the default color scheme; Editor settings can switch to the
bundled light scheme, load a custom TOML theme or font, and change the editor
font family and size. See [`themes/README.md`](themes/README.md) for the theme
format. Greek shortcut expansion is enabled by default, turning inputs such as
`\pi`, `\gamma`, or `\theta` into `π`, `γ`, or `θ`; it can be disabled from the
editor toolbar. Unicode identifiers are valid source; `π` and
`τ` are built-in constant aliases and `Γ` calls the Gamma function. Use
Ctrl+Enter to run and Ctrl+S to save the current file.

The editor offers bounded, prefix-based completion for language constructs and
every built-in function. Press Tab to accept the first suggestion, or use the
**Snippets** menu for render blocks, documented equations, functions, control
flow, matrices, and step-by-step linear algebra. Completion is local and does
not run the parser or evaluator while typing, and suggestions appear directly
under the text cursor.

`scas --help` lists decimal-precision and execution-limit flags; `--version`
prints the release version. Redirection and pipelines remain supported.

### Current examples

The executable examples under [`tests/should_pass`](tests/should_pass) are
checked on every test run:

| Example | Demonstrates |
| --- | --- |
| `language_tour.scas` | exact arithmetic, symbolic functions, `else if`, and semicolon-style `for` |
| `symbolic_algebra.scas` | collection, expansion, factoring, partial fractions, Horner form, solving, and identities |
| `calculus_and_domains.scas` | assumptions, substitution, derivatives, integrals, and unevaluated fallbacks |
| `matrices_and_closures.scas` | matrix literals, comma indexing, arrays, and lexical closures |
| `documented_calculation.scas` | `///` prose, render blocks and lines, and `=>` markers |

Successful batch programs exit with status 0. Input, lexical, syntax, and
runtime failures are written to standard error and exit with status 1. Syntax
diagnostics include the source line and exact failing span:

```text
syntax error: expected an expression
  --> <stdin>:1:11
   |
 1 | print(1 + );
   |           ^ unexpected `)`
```

Scanning reports all lexical errors found in one pass. Parsing and execution
stop at the first syntax or runtime error. Diagnostics have stable phase labels:
`lexical error`, `syntax error`, `runtime error`, and `limit error`. Runtime
diagnostics point to the statement that failed.

## Language basics

Most simple statements end with a semicolon. Line comments begin with `//`.
Declarations, assignments, `print`, `return`, `break`, `continue`, expression
statements, and expression-bodied functions require a trailing semicolon.
Blocks do not. A `for` header uses C-style semicolon separators:
`for initializer; condition; update { ... }`; the update is not followed by a
fourth semicolon.

```scas
// Exact arithmetic and symbolic expressions coexist.
let probability = 1 / 3 + 1 / 6;
let model = 2x^2 - 3x + 1;
let samples = [1, 2, 3; 4, 5, 6];

print(probability);     // 1/2
print(factor(model));   // (x - 1/2) * (x - 1) * 2
print(samples[1, 2]);   // 6
```

Variables declared without a value are initialized to `nil`:

```scas
let result;
```

Functions may have a concise expression body or a normal block body. Block
functions return `nil` when execution reaches the end without `return`:

```scas
fn gradient_step(value, gradient, rate) = value - rate * gradient;

fn make_shift(offset) {
    fn shift(value) = value + offset;
    return shift;
}

fn classify(score) {
    if score >= 90 {
        return "excellent";
    } else if score >= 75 {
        return "strong";
    }
    return "developing";
}

let shift_by_five = make_shift(5);
print(gradient_step(10, 4, 1 / 2)); // 8
print(shift_by_five(3));            // 8
print(classify(87));                // strong
```

For mathematical functions, the `fn` keyword is optional when using an
expression body. Adjacent terms imply multiplication, and unbound identifiers
become symbolic values. Calling a function with symbolic arguments substitutes
them while still evaluating every numeric part:

```scas
f(x) = x^3 - 3x + 1;

print(f);                    // f(x) = x ^ 3 - 3 * x + 1
print(derivative(f(x), x));  // 3 * x ^ 2 - 3
print(subs(f(x), x, 2));     // 3
print(2(x + 1));             // 2 * (x + 1)
```

Conditionals and loops use braces:

```scas
let sum_of_squares = 0;

for let i = 1; i <= 5; i += 1 {
    sum_of_squares += i^2;
}

if sum_of_squares > 50 {
    print("large");
} else if sum_of_squares > 25 {
    print("medium");
} else {
    print("small");
}

print(sum_of_squares); // 55
```

Blocks create a new scope. A variable declared inside `{ ... }` is not
available outside that block.

Conditions use value truthiness: `false` and `nil` are false; every other value
is true. `and` and `or` short-circuit and return one of their operands. Equality
is structural for arrays and exact across numeric representations (for
example, `1 / 2 == 0.5`). Functions and blocks use lexical scope.

Exponentiation is right-associative; all other binary operators are
left-associative. From tightest to loosest, precedence is postfix operations,
exponentiation, unary operators, multiplication/division/remainder, addition/
subtraction, shifts, bitwise `&`, `xor`, bitwise `|`, comparisons, equality,
`and`, then `or`. Adjacency (`2x`, `2(x + 1)`, `(x + 1)y`) means
multiplication. A named function call such as `f(x)` takes precedence over
adjacency; a numeric value is not callable, so `2(x)` is multiplication.

The experimental self-documenting grammar recognizes `///` prose, shared-scope
render blocks `\{ ... \}`, render lines `\[ expression \]`, and the `=>`
show-result marker. These forms execute today. The source-expression renderer
is available as a library API, while whole-document assembly and evaluated
`=>` output remain tracked in [`docs/DOCUMENTER.md`](docs/DOCUMENTER.md).

```rust
use scas::ast::StatementKind;
use scas::latex::LatexRenderer;
use scas::lexer::Scanner;
use scas::parser::Parser;

let mut scanner = Scanner::new("print(integral(sin(2x), x));");
let tokens = scanner.scan_tokens().clone();
let symbols = scanner.into_symbols();
let program = Parser::new(tokens).parse().unwrap();
let StatementKind::Print(expression) = &program[0].kind else {
    unreachable!();
};

assert_eq!(
    LatexRenderer::new(&symbols).expression(expression),
    r"\int \sin\left(2x\right)\,dx",
);
```

Rendering writes directly into one output buffer without intermediate
expression strings. `expression_into` accepts a caller-owned `String` so a live
preview can reuse its allocation. The renderer currently covers precedence,
fractions, powers, roots, implicit products, calculus notation, Greek names,
subscripts, vectors, matrices, and raw LaTeX strings.

## Values and operators

SCAS has integers, rationals, decimals, complex numbers, symbolic expressions, arrays, strings,
booleans (`true` and `false`), `nil`, and functions. Integer and decimal
literals can be arbitrarily large. Integers may use `0b`, `0o`, and `0x`
prefixes, while decimals support scientific notation such as `1.25e+40`.

Integer division produces a reduced exact rational. A decimal operand requests
decimal division instead:

```scas
print(1 / 3);      // 1/3
print(6 / 3);      // 2 (a denominator of one normalizes to an integer)
print(1.0 / 3);    // decimal, rounded using the active precision
print(1 / 2 == 0.5); // true
```

The real numeric promotion order is `Integer -> Rational -> Decimal`; a complex
operand promotes the operation to `Complex`. Integer and rational arithmetic is
exact. Decimal addition, subtraction, multiplication, and positive integral
powers preserve all digits. Division and non-integral powers use the active
significant-digit precision, which defaults to 100 and can be changed in the
REPL with `:precision N`.

## Resource limits

Batch execution uses deterministic defaults to prevent runaway programs:
1,000,000 source characters, 250,000 tokens, 256 nested delimiters, 1,000,000
evaluation steps, 256 nested function calls, exponent magnitude 100,000, shift
amount 1,000,000, decimal precision 10,000 digits, transcendental precision
2,000 digits, decimal scale 100,000, and numeric values 8,000,000 bits. Crossing a budget
returns a `limit error`; it never terminates the process by panic. Embedders can
override every value with `execute_with_limits(source, output, Limits { ... })`.
The REPL applies the same execution, recursion, numeric, and precision defaults
to each submission.

## Standard mathematics and algebra

SCAS provides exact symbolic constants, arbitrary-precision elementary
functions, and real special functions:

- Constants: `pi`, `e`, `tau`
- Powers and logs: `sqrt`, `cbrt`, `exp`, `ln`, `log`, `log10`, `log2`
- Trigonometry: `sin`, `cos`, `tan`, `sec`, `csc`, `cot`, `asin`, `acos`,
  `atan`, `atan2`
- Hyperbolic functions: `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`
- Special functions: `erf`, `erfc`, `gamma`, `lgamma`
- Utilities: `abs`, `sign`, `floor`, `ceil`, `round`, `trunc`, `min`, `max`,
  `hypot`, `gcd`, `lcm`, `degrees`, `radians`

`log(x)` and `ln(x)` are natural logarithms; `log(x, base)` uses an explicit
base. Exact inputs stay exact or symbolic: `sqrt(4)` is `2`, `sin(pi/6)` is
`1/2`, and `sqrt(2)` remains `sqrt(2)`. Use `evalf(expression)` to request a
decimal approximation at the active precision. Decimal inputs request numeric
evaluation directly.

Elementary functions use arbitrary-precision decimal real and complex
arithmetic; they never pass through a machine `f64`. Results outside the real
domain are promoted to principal complex values, so `sqrt(-1)` is `i` and
`evalf(ln(-1))` is `pi*i` to the requested precision. `gamma`, `lgamma`, `erf`,
and `erfc` are arbitrary-precision real functions; complex arguments to those
four functions are explicitly unsupported. See
[`docs/math-functions.md`](docs/math-functions.md) for branch conventions,
exactness, precision, and the complete support table.

Symbolic inputs remain symbolic and safe elementary identities are applied:

```scas
print(sqrt(x^2)); // retained: an unconstrained symbol may be complex
assume_real(x);
print(sqrt(x^2)); // abs(x)
assume_nonnegative(x);
print(sqrt(x^2)); // x
print(sin(t)^2 + cos(t)^2); // 1
```

Polynomial and rational-expression tools are explicit so ordinary
simplification does not unexpectedly expand an expression:

```scas
print(collect(x*x + 2x + x^2, x)); // 2*x^2 + 2*x
print(expand((x + y)^3));           // canonical polynomial expansion
print(factor(x^3 - y^3));           // difference of cubes
print(apart(1 / (x^2 - 1)));        // partial fractions
print(horner(2x^3 - 6x^2 + 1));     // Horner form
print(subs(x^2 + 2x + 1, x, 3));    // 16
print(solve(x^2 - 5x + 6, x));      // [2, 3]
print(solve(x^2 + 1, x));            // [-i, i]
print(solve(x^3 - 2, x));            // all three approximate complex roots
print(solve(sin(x) - 1/2, x));       // principal solutions
print(solve(exp(2x + 1) - 7, x));    // [(ln(7) - 1) / 2]
print(solve(2^x - 8, x));            // [log(8, 2)]
print(derivative(x^3 + sin(x), x));  // 3*x^2 + cos(x)
print(integral(sqrt(x), x));         // 2*sqrt(x)^3/3
```

`expand` uses a canonical sparse multivariate polynomial representation.
`factor` performs content and common-monomial extraction, square/cube
identities, square-free decomposition, and rational-root factorization.
`apart` currently supports proper rational functions whose denominator splits
into distinct rational linear factors.

`solve` keeps rational roots exact, uses the exact quadratic formula for real
and complex quadratics, and finds every complex root numerically for remaining
univariate polynomial factors through degree 64. It also inverts affine-argument
trigonometric, hyperbolic, exponential, and logarithmic expressions. Trig
results are the distinct principal branches; periodic integer-parameter
solution families are not yet represented by the language's array value.

The GUI's resizable **LaTeX preview** pane updates from `///` prose and
`\[ expression \]` render lines. It shows a readable math view, exposes the
generated LaTeX for copying, and retains the last valid document while an
expression is incomplete. **Export TeX** writes a complete document beside the
current source file, and **Export PDF** invokes `pdflatex` on that document.

Live editing uses cached, serialized system `latex`/`dvipng` rendering by
default so the preview retains Computer Modern and faithful TeX layout. A fast
structured renderer remains available as a fallback and can be selected by
disabling **Computer Modern**. Analysis jobs are debounced, serialized, and
evaluated with a bounded preview budget so stale expensive calculations cannot
accumulate background workers. Explicit Run and export actions keep the normal
full computation budget.

Arrays are immutable, dynamically typed, and may be nested. Indexes are
zero-based non-negative integers. `+` concatenates two arrays:

```scas
let values = [1, 1 / 2, "three", [4, 5]];
print(values[1]);       // 1/2
print(values[3][0]);    // 4
print(values + [6, 7]);

fn second(xs) = xs[1];
print(second([10, 20]));

let matrix = [1, 2; 3, 4];
print(matrix[1, 0]); // 3; equivalent to matrix[1][0]
print(det(matrix));
print(inverse(matrix));
// In a render block, `rref(matrix) =>;` documents every row operation.
print(rref_steps(matrix)); // explicit structured-work form; prints the final result
```

The editor expands the complete standard Greek alphabet and common variant
forms such as `\vartheta` and `\varphi`. `\pi`, `\tau`, and `\phi` resolve to
the reserved exact constants; other Greek letters remain ordinary symbols.

| Category | Operators |
| --- | --- |
| Arithmetic | `+`, `-`, `*`, `/`, `%`, `^` |
| Comparison | `==`, `!=`, `<`, `<=`, `>`, `>=` |
| Logical | `!`, `and`, `or` |
| Bitwise (integers) | `~`, `&`, `xor`, `|`, `<<`, `>>` |
| Assignment | `=`, `+=`, `-=`, `*=`, `/=`, `%=`, `^=`, `&=`, `|=` |

The `+` operator adds numbers, joins two strings, or concatenates two arrays.
Bitwise operations require integers. Integral exponents use exact arithmetic;
exact non-integral powers stay symbolic when no exact root exists, while a
decimal exponent requests an arbitrary-precision numeric result. Every
value except `false` and `nil` is treated as true in a condition. Logical `and`
and `or` short-circuit.

## Using SCAS as a Rust library

The crate exposes a pipeline that scans, parses, and executes a complete source
program without printing or terminating the host process. Program output is
written to a caller-provided `Write` implementation:

```rust
let source = "print(1 / 2 + 1 / 3);";
let mut output = Vec::new();

if let Err(error) = scas::execute(source, &mut output) {
    eprintln!("{}", error.render(source, "example.scas"));
}

assert_eq!(String::from_utf8(output).unwrap(), "5/6\n");
```

`execute` creates a fresh environment for each call. `execute_with_options`
configures decimal precision and resource limits. Embedders that need a
persistent session can use the public `Scanner`, `Parser`, and `Interpreter`
types directly. Interpreter output is injected through a `Write`
implementation; it never relies on global standard output.

SCAS follows Semantic Versioning. While the crate is `0.x`, public library APIs
may change in a minor release; breaking changes are called out in the changelog
and receive a deprecation cycle when practical. Patch releases preserve the
public API, and MSRV changes are documented as minor-version changes.

## Development

The same checks enforced in CI can be run locally with:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo bench --bench baselines
cargo +nightly fuzz run scanner -- -max_total_time=30
cargo +nightly fuzz run parser -- -max_total_time=30
```

CI runs these checks on the minimum supported Rust release (1.79) and the
latest stable release. Programs under `tests/should_pass` and
`tests/should_fail` are integration fixtures with matching `.stdout` or
`.stderr` golden files; adding a `.scas` fixture without its expected output
causes the test suite to fail.

Grammar notes are available in [`src/grammar.org`](src/grammar.org), and
additional example programs are under [`tests/should_pass`](tests/should_pass).
The symbolic simplification roadmap is documented in
[`docs/algebraic_identities.md`](docs/algebraic_identities.md).
See also the [language reference](docs/language.md), [error guide](docs/errors.md),
[architecture notes](docs/architecture.md), and [contribution guide](CONTRIBUTING.md).
Measured development baselines are recorded in
[`docs/performance.md`](docs/performance.md).
