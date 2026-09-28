# SCAS

SCAS is a dynamically typed interpreted language and an early-stage computer
algebra system written in Rust. It supports arbitrary-size integers, exact
rationals, arbitrary-precision decimals, arrays, closures, control flow, and
numeric and bitwise operators.

## Requirements

- Rust 1.79 or newer with Cargo ([install Rust](https://rustup.rs/))

## Running a program

Running SCAS in a terminal starts an interactive REPL. Definitions remain
available until the session ends, bare expressions print their values, and
multiline blocks receive a continuation prompt.

```text
$ cargo run --quiet
scas 0.1.0 — type :help for help, :quit to exit
> 1 / 2 + 1 / 3
5/6
> :precision 200
decimal precision: 200 digits
```

Use `:help`, `:clear`, `:precision N`, or `:quit` to control the REPL.

When standard input is redirected, SCAS reads and executes a complete program.
From the repository root, run a `.scas` file with:

```sh
cargo run --quiet < path/to/program.scas
```

For example, the included sample can be run with:

```sh
cargo run --quiet < tests/should_pass/test1.scas
```

You can also pipe a short program directly into the interpreter:

```sh
printf 'print(2 ^ 8);\n' | cargo run --quiet
```

To build once and run the compiled executable:

```sh
cargo build --release
./target/release/scas < path/to/program.scas
```

Successful batch programs exit with status 0. Input, lexical, syntax, and
runtime failures are written to standard error and exit with status 1. Syntax
diagnostics include the source line and exact failing span:

```text
error: expected an expression
  --> <stdin>:1:11
   |
 1 | print(1 + );
   |           ^ unexpected `)`
```

## Language basics

Most simple statements end with a semicolon. Line comments begin with `//`.

```scas
// Variables and values
let name = "SCAS";
let count = 3;
let enabled = true;
let nothing = nil;

// Reassignment and compound assignment
count += 2;
count *= 4;

// Output
print(name + " example");
print(count);
```

Variables declared without a value are initialized to `nil`:

```scas
let result;
```

Functions may have a concise expression body or a normal block body. Block
functions return `nil` when execution reaches the end without `return`:

```scas
fn add(a, b) = a + b;
fn square(x) = x ^ 2;

fn factorial(n) {
    if n <= 1 {
        return 1;
    }
    return n * factorial(n - 1);
}

let answer = add(square(4), 5);
print(answer);
```

For mathematical functions, the `fn` keyword is optional when using an
expression body. Adjacent terms imply multiplication, and unbound identifiers
become symbolic values. Calling a function with symbolic arguments substitutes
them while still evaluating every numeric part:

```scas
f(x, y) = 2x + 3y;

print(f);           // f(x, y) = 2 * x + 3 * y
print(f(x ^ 2, y)); // 2 * x ^ 2 + 3 * y
print(f(2, y));     // 4 + 3 * y
print(f(2, 5));     // 19
print(2(x + 1));    // 2 * (x + 1)
```

Conditionals and loops use braces:

```scas
let total = 0;

for let i = 0; i < 5; i += 1 {
    if i % 2 == 0 {
        total += i;
    } else {
        continue;
    }
}

while total < 10 {
    total += 1;
    if total == 8 {
        break;
    }
}

print(total);
```

Blocks create a new scope. A variable declared inside `{ ... }` is not
available outside that block.

## Values and operators

SCAS has integers, rationals, decimals, symbolic expressions, arrays, strings,
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

The numeric promotion order is `Integer -> Rational -> Decimal`. Integer and
rational arithmetic is exact. Decimal addition, subtraction, multiplication,
and positive integral powers preserve all digits. Division and negative decimal
powers use the active significant-digit precision, which defaults to 100 and
can be changed in the REPL with `:precision N`.

## Standard mathematics and algebra

SCAS provides real-valued mathematical constants and functions:

- Constants: `pi`, `e`, `tau`
- Powers and logs: `sqrt`, `cbrt`, `exp`, `ln`, `log`, `log10`, `log2`
- Trigonometry: `sin`, `cos`, `tan`, `sec`, `csc`, `cot`, `asin`, `acos`,
  `atan`, `atan2`
- Hyperbolic functions: `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`
- Special functions: `erf`, `erfc`, `gamma`, `lgamma`
- Utilities: `abs`, `sign`, `floor`, `ceil`, `round`, `trunc`, `min`, `max`,
  `hypot`, `gcd`, `lcm`, `degrees`, `radians`

`log(x)` and `ln(x)` are natural logarithms; `log(x, base)` uses an explicit
base. Exact special values such as `sqrt(4)`, `exp(0)`, and `cos(0)` remain
exact. General transcendental results currently use real `f64` evaluation and
are converted to a 16-significant-digit decimal. Inputs outside the real domain
produce runtime errors rather than complex results.

Symbolic inputs remain symbolic and safe elementary identities are applied:

```scas
print(sqrt(x^2)); // abs(x)
print(sin(-x));   // -sin(x)
print(cos(-x));   // cos(x)
```

Polynomial and rational-expression tools are explicit so ordinary
simplification does not unexpectedly expand an expression:

```scas
print(expand((x + y)^2));        // x^2 + 2*x*y + y^2
print(factor(x^2 - y^2));        // (x-y)*(x+y)
print(apart(1 / (x^2 - 1)));     // partial fractions
print(horner(2x^3 - 6x^2 + 1));  // Horner form
```

`expand` uses a canonical sparse multivariate polynomial representation.
`factor` performs content and common-monomial extraction, square/cube
identities, square-free decomposition, and rational-root factorization.
`apart` currently supports proper rational functions whose denominator splits
into distinct rational linear factors.

Arrays are immutable, dynamically typed, and may be nested. Indexes are
zero-based non-negative integers. `+` concatenates two arrays:

```scas
let values = [1, 1 / 2, "three", [4, 5]];
print(values[1]);       // 1/2
print(values[3][0]);    // 4
print(values + [6, 7]);

fn second(xs) = xs[1];
print(second([10, 20]));
```

| Category | Operators |
| --- | --- |
| Arithmetic | `+`, `-`, `*`, `/`, `%`, `^` |
| Comparison | `==`, `!=`, `<`, `<=`, `>`, `>=` |
| Logical | `!`, `and`, `or` |
| Bitwise (integers) | `~`, `&`, `xor`, `|`, `<<`, `>>` |
| Assignment | `=`, `+=`, `-=`, `*=`, `/=`, `%=`, `^=`, `&=`, `|=` |

The `+` operator adds numbers, joins two strings, or concatenates two arrays.
Bitwise operations require integers, and exponents must be integers. Every
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

`execute` creates a fresh environment for each call. Embedders that need a
persistent session or custom decimal precision can use the public `Scanner`,
`Parser`, and `Interpreter` types directly. Interpreter output is also injected
through a `Write` implementation; it never relies on global standard output.

## Development

The same checks enforced in CI can be run locally with:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
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
