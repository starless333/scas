# SCAS

SCAS is a small interpreted language written in Rust. It currently supports
variables, expression-based functions, control flow, strings, booleans, and
numeric and bitwise operators.

## Requirements

- A recent Rust toolchain with Cargo ([install Rust](https://rustup.rs/))

## Running a program

SCAS reads the complete program from standard input. From the repository root,
run a `.scas` file with:

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

Functions have a single expression as their body. Both the declaration and
ordinary function calls used as statements end with a semicolon:

```scas
fn add(a, b) = a + b;
fn square(x) = x ^ 2;

let answer = add(square(4), 5);
print(answer);
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

SCAS currently has numbers, strings, booleans (`true` and `false`), `nil`, and
functions. Numbers may be decimal or use `0b`, `0o`, and `0x` prefixes.

| Category | Operators |
| --- | --- |
| Arithmetic | `+`, `-`, `*`, `/`, `%`, `^` |
| Comparison | `==`, `!=`, `<`, `<=`, `>`, `>=` |
| Logical | `!`, `and`, `or` |
| Bitwise (integers) | `~`, `&`, `xor`, `|`, `<<`, `>>` |
| Assignment | `=`, `+=`, `-=`, `*=`, `/=`, `%=`, `^=`, `&=`, `|=` |

The `+` operator adds numbers or joins two strings. Every value except `false`
and `nil` is treated as true in a condition. Logical `and` and `or`
short-circuit.

## Development

Build and run the Rust test suite with:

```sh
cargo test
```

Grammar notes are available in [`src/grammar.org`](src/grammar.org), and
additional example programs are under [`tests/should_pass`](tests/should_pass).
