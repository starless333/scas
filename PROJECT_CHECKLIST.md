# SCAS project checklist

This checklist records gaps found in a repository audit on 2026-09-28. It is
ordered by priority: finish P0 before relying on SCAS for unattended programs,
then work downward. A checkbox should only be marked complete when its stated
tests or acceptance criteria are also complete.

## P0 — correctness and failure behavior

- [x] Replace parser panics with structured, recoverable errors.
  - Return `Result` from parsing instead of calling `panic!` in
    `src/parser/parser.rs`.
  - Include the offending token, line, and column in each diagnostic.
  - Remove the REPL's `catch_unwind` and global panic-hook manipulation once
    parse failures are ordinary errors.
  - Confirm malformed piped input reports a concise error without a Rust panic
    or backtrace.
- [x] Give the executable reliable exit statuses and I/O error handling.
  - Exit nonzero on lexing, parsing, runtime, and stdin-read failures.
  - Do not `unwrap()` stdin reads or other user-controlled operations.
  - Add CLI tests that assert stdout, stderr, and the process status for every
    failure class. Currently a runtime error such as division by zero exits 0.
- [x] Turn the program fixtures into real integration tests.
  - Execute every file under `tests/should_pass` and `tests/should_fail` from
    `cargo test`; merely storing the files does not test them.
  - Give passing fixtures deterministic expected output and failing fixtures
    expected diagnostics/statuses.
  - Repair or reclassify stale fixtures: `should_pass/fib.scas` and
    `should_pass/test2.scas` currently panic, while `should_fail/test3.scas`
    currently succeeds.
- [x] Add regression tests for all operators and runtime errors.
  - Cover precedence/associativity, implicit multiplication, compound
    assignment, control flow, closures/scopes, arrays/indexing, and function
    arity.
  - Cover zero division/remainder, invalid shifts and exponents, out-of-range
    indexing, invalid operand types, undefined assignment, and top-level
    `return`/`break`/`continue`.
  - Assert exact results around integer/rational/decimal promotion and the
    configured decimal precision.

## P1 — a dependable development baseline

- [x] Add continuous integration for the supported Rust toolchain(s).
  - Run `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features
    -- -D warnings`, and `cargo test` on every change.
  - Fix the current formatting diff and Clippy failures before making those
    checks required.
  - Pin a minimum supported Rust version or a toolchain policy and test it.
- [x] Split reusable language code from the executable.
  - Move lexer, parser, AST, number, and interpreter modules into `src/lib.rs`;
    keep terminal/process behavior in a thin binary entry point.
  - Expose one pipeline API that scans, parses, and executes source without
    printing or terminating the process.
  - Inject output into the interpreter so tests and embedders do not depend on
    global `println!`.
- [x] Make diagnostics consistent across scanner, parser, interpreter, REPL,
  and batch mode.
  - Attach source spans to tokens and AST errors and render source snippets.
  - Use stable diagnostic categories/messages that integration tests can
    verify.
  - Define whether execution stops at the first error or recovers to report
    more errors.
- [x] Reconcile the grammar, implementation, examples, and fixtures.
  - Choose and document the `for` separator syntax; `src/grammar.org` uses
    commas while the parser and README use semicolons.
  - Remove or implement grammar-only syntax (for example `~=`), and document
    implementation-only behavior.
  - Specify semicolon rules, implicit-multiplication ambiguities, operator
    associativity, scope, truthiness, equality, and numeric conversion.
- [x] Add focused unit and property tests below the REPL layer.
  - Test scanner tokens/spans, parser ASTs, environments, numeric operations,
    value equality/formatting, and symbolic normalization independently.
  - Add randomized invariants for exact arithmetic and safe algebraic
    identities, including zero and negative boundary cases.
- [x] Define and enforce resource limits.
  - Handle excessive recursion, deeply nested input, huge exponents/shifts,
    enormous decimal precision, and non-terminating programs without stack
    overflows or accidental unbounded allocation.
  - Document defaults and provide explicit overrides where appropriate.

## P2 — usable CAS and language capabilities

- [x] Establish a canonical symbolic-expression representation before adding
  many more rewrite rules.
  - Flatten associative sums/products, deterministically order operands, and
    normalize coefficients and signs.
  - Separate `simplify`, `expand`, and `factor`, with a termination measure or
    rewrite budget.
  - Track assumptions/domains so cancellation and roots do not silently change
    meaning.
  - Work through the detailed identity backlog in
    [`docs/algebraic_identities.md`](docs/algebraic_identities.md).
- [x] Add the core operations expected from a computer algebra system.
  - Built-in constants and functions (`pi`, `e`, `abs`, roots, exponential,
    logarithmic, and trigonometric functions).
  - Polynomial collection, expansion, factoring, substitution, and solving.
  - Symbolic differentiation, then integration with an unevaluated fallback.
  - Document numeric domains (real/complex), branch conventions, and behavior
    when a result is unknown.
- [x] Provide a conventional command-line interface.
  - Accept a source-file path as well as stdin, plus `--help` and `--version`.
  - Add flags for decimal precision and relevant execution/resource limits.
  - Keep REPL prompts/colors disabled automatically for non-terminal output.
- [x] Improve the interactive experience.
  - Add history and line editing, cursor-aware multiline input, and graceful
    interrupt/EOF behavior.
  - Add commands for loading files, inspecting definitions, and resetting
    state without terminal escape sequences leaking to redirected output.
- [x] Complete user-facing documentation.
  - Add a language reference derived from the tested grammar, an error guide,
    and examples for every supported feature.
  - Clearly label experimental behavior and unsupported CAS operations.
  - Add architecture and contributor documentation for maintainers.
- [x] Complete package and project metadata.
  - Choose and add a license, then fill in Cargo description, license,
    repository, documentation/homepage, keywords, and categories.
  - Add `CONTRIBUTING.md`, a changelog/release policy, and a code of conduct if
    outside contributions are intended.

## P3 — hardening, performance, and distribution

- [x] Add fuzzing for the scanner/parser and property-based differential tests
  for numeric and symbolic evaluation; preserve every discovered crash as a
  regression test.
- [x] Add benchmarks for scanning, parsing, large-number arithmetic, symbolic
  normalization, function calls, and representative end-to-end programs.
- [x] Profile before optimizing, then set documented performance and memory
  baselines so regressions are visible in CI.
- [x] Add release automation, signed/versioned artifacts, checksums, and
  installation instructions for supported platforms.
- [x] Generate API documentation for the library and publish it once the public
  API has stability expectations.

## Current verification snapshot

- [x] `cargo test --all-targets --all-features` passes (53 tests).
- [x] `cargo fmt --all -- --check` passes.
- [x] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [x] File-based pass/fail fixtures are exercised by `cargo test`.
- [x] Invalid programs fail cleanly and consistently without panicking.
- [x] Failures return a nonzero process exit status.
