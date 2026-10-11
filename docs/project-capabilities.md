# SCAS project capabilities

SCAS is a from-scratch scripting language and computer algebra system written
in Rust. It combines a Unicode-aware language toolchain, exact and
arbitrary-precision mathematics, symbolic algebra, executable mathematical
documents, a command-line workflow, an LSP server, a native desktop IDE, and a
Wasm browser IDE.

This document inventories behavior implemented in the repository today. The
final section states the important boundaries; proposed work belongs in
[`TODO_STILL.md`](TODO_STILL.md) and [`PRIORITY.md`](PRIORITY.md).

## Executive overview

SCAS owns its evaluation pipeline end to end. Source is scanned into spanned
tokens, parsed into an AST, evaluated in linked lexical environments, and
reported through phase-specific diagnostics. The numeric layer preserves exact
integers and rationals, supports arbitrary-precision decimal and complex
calculation, and enters approximation explicitly through `evalf`. The symbolic
layer provides domain-aware simplification, polynomial algorithms, equation
solving, calculus, and linear algebra.

One core supports five user-facing surfaces:

- a batch CLI for files, standard input, and document builds;
- a persistent terminal REPL;
- an LSP server for external editors;
- a native `eframe`/`egui` IDE;
- a static Wasm browser IDE whose evaluation runs in a Web Worker.

The project uses its own evaluator and symbolic engine. SymPy is used only by
an optional differential test script and fixed-workload benchmark, not at
runtime.

## Language and runtime

- Dynamically typed values: arbitrary-size integers, reduced rationals,
  arbitrary-precision decimals and complex numbers, symbolic expressions,
  strings, booleans, `nil`, infinities, arrays, maps, functions, and builtins.
- Declarations, ordinary and compound assignment, expressions, printing,
  lexical blocks, `if`/`else`, `while`, C-style `for`, `break`, `continue`, and
  `return`.
- Expression-bodied mathematical functions and block-bodied functions.
- First-class functions and closures over linked lexical environments.
- Unicode identifiers and executable aliases such as `π`, `τ`, `φ`, and `Γ`.
- Implicit multiplication such as `2x`, `3(x + 1)`, and `(x + 1)y`, with
  explicit parser precedence.
- Zero-based indexing for arrays and matrices, including `A[row, column]`.
- Structural array and map equality, string and array concatenation,
  short-circuit logical operators, and integer bitwise operations and shifts.
- First-class maps with scalar keys plus `map`, `put`, `keys`, and `values`.
- String formatting through `{}` fields with escaped-brace support.
- `import` with shared module scope and a bundled SCAS standard library in
  [`stdlib/core.scas`](../stdlib/core.scas).
- Host-independent I/O builtins: `read_text`, `write_text`, `input`, `args`,
  `read_csv`, and `read_json`. Native execution uses the filesystem and process
  environment; the browser supplies a virtual filesystem and prompts.
- Runtime introspection through `type_of` and builtin-oriented `help`.
- Reusable Rust APIs for one-shot execution and persistent interpreters, with
  caller-owned output, configurable precision, host I/O, and resource limits.

The tested grammar is summarized in [`language.md`](language.md); the normative
production grammar is [`src/grammar.org`](../src/grammar.org).

## Numeric computation

- Exact arbitrary-size integer and rational arithmetic.
- Arbitrary-precision decimal arithmetic with configurable significant digits
  and guard digits for iterative algorithms.
- Arbitrary-precision complex arithmetic for elementary functions and powers,
  using documented principal branches.
- Exact symbolic constants `pi`, `e`, `tau`, and `phi`.
- Square and cube roots; exponential and logarithmic functions; circular,
  inverse, and hyperbolic trigonometry; angle conversion; rounding; magnitude;
  sign; `min`/`max`; `hypot`; GCD; and LCM.
- Real and complex numerical paths for `gamma`, `lgamma`, `erf`, and `erfc`
  using recurrence, reflection, series, and asymptotic methods.
- Exact common-angle trigonometric values and symbolic retention when SCAS
  cannot justify an exact result.
- `evalf` as the explicit boundary from exact or symbolic values to requested
  numerical approximation.
- Checked errors for poles, invalid domains or dimensions, non-finite results,
  convergence failure, and exhausted precision or evaluation budgets.

Branch and precision behavior is documented in
[`math-functions.md`](math-functions.md), and the generated callable reference
is [`builtins.html`](builtins.html).

## Symbolic algebra

- Immutable, reference-counted expression trees for numbers, constants,
  variables, unary and binary operations, and builtin calls.
- Conservative simplification that does not silently assume an unknown symbol
  is real, positive, integral, or nonzero.
- Named assumptions for real, integer, nonzero, nonnegative, positive, and
  negative symbols.
- General `assume(constraint)` facts for supported equalities, inequalities,
  conjunctions, bounds, and expressions, with contradiction detection and
  `forget_assumptions()` reset.
- Canonical sparse multivariate polynomial conversion for supported algebraic
  expressions.
- `simplify`, `expand`, `collect`, `horner`, substitution, content extraction,
  and common-monomial and special-identity factoring.
- `together` for combining rational expressions and `apart` for improper
  fractions, repeated rational linear factors, and powers of one remaining
  irreducible quadratic factor.
- Rational-root and square-free factoring, exact quadratic factoring, exact
  monic cubic divisor search, and verified reconstruction for a bounded family
  of degree 4–10 integer polynomials. Candidate factors are accepted only after
  exact rational division.
- Exact univariate polynomial degree, coefficient extraction, GCD, quotient
  and remainder, resultant, and discriminant.
- A user rule system: `rewrite(expr, pattern, replacement)` or a reusable rule
  array performs a finite top-down pass with consistent wildcard bindings,
  first-match order, and supported sign/nonzero guards.

## Solving and verification

- Exact rational, quadratic, and complex quadratic roots.
- Numerical complex roots for remaining supported univariate polynomial
  factors through degree 64.
- Inversion of supported affine elementary functions for principal solutions.
- `solve_periodic(expr, x, n)` for integer-parameter sine, cosine, and tangent
  families with affine arguments.
- `assert` and its `verify` alias for executable mathematical claims.
- `verify_chain([step1, step2, ...])`, which checks each adjacent pair through
  exact polynomial arithmetic or identical symbolic structure and reports the
  first unproved link.

## Calculus

- Symbolic differentiation of algebraic powers and roots, exponential and
  logarithmic functions, circular and hyperbolic functions, inverse functions,
  error functions, and assumption-qualified absolute values.
- Multivariable `gradient`, `jacobian`, and `hessian`.
- Rule-based indefinite integration for polynomial powers, affine
  substitutions, reciprocals, roots, exponentials, logarithms, trigonometric
  and hyperbolic families, selected inverse/special functions, and bounded
  integration-by-parts families.
- Rational integration through the implemented partial-fraction families,
  including a bounded recurrence for positive-definite quadratic powers with
  linear numerators.
- Definite integration through the current antiderivative rules, with
  linearity and even/odd symmetry on supported intervals.
- Multiple integrals represented through repeated supported integrations.
- Finite and infinite rational-function limits using polynomial valuations and
  degree comparison, plus directional affine absolute-value cases and bounded
  L'Hopital evaluation.
- Taylor and Laurent series for supported rational-polynomial expressions,
  through order 64, with optional explicit big-O output.
- Exact finite sums and products; closed forms for supported polynomial,
  geometric, and `k*r^k` sums.
- Basic ODE solving for separable equations, first-order linear equations, and
  homogeneous constant-coefficient equations through second order.
- Unevaluated symbolic integrals or limits when no safe implemented rule can
  establish a result.

## Linear algebra

- Exact or symbolic transpose, trace, determinant, rank, RREF, inverse,
  matrix multiplication, and unique linear-system solving.
- Natural matrix multiplication using `A * B` and matrix-vector `A * v`, plus
  the explicit `matmul` builtin and dimension validation.
- Postfix transpose notation (`A'`) equivalent to `transpose(A)`.
- Square-matrix integer powers by exponentiation by squaring, including the
  identity at exponent zero and exact inverse powers for negative exponents.
- Vector dot product, cross product, and Euclidean norm.
- Characteristic polynomials, eigenvalues, eigenvectors, and paired
  `eigenpairs` results.
- Exact spectral results for 1×1 and 2×2 matrices and supported triangular
  matrices; larger real symmetric matrices use numeric Jacobi iteration, and
  general numeric matrices through 6×6 use residual-checked complex
  eigenpairs.
- Natural, ascending, and descending eigenpair ordering without losing the
  value/vector association; numeric ordering is restricted to real values.
- SVD returning `[U, Sigma, Vt]` for real or complex numeric rectangular
  matrices. Real 1×1 and 2×2 square cases retain exact arithmetic.
- LU, QR, and Cholesky decompositions; null and column spaces; Gram-Schmidt;
  projection; change of basis; matrix exponential; and bounded Jordan form.
- Step-producing variants for determinant, RREF, inverse, multiplication,
  linear solving, transpose, trace, rank, vector operations, LU, QR, and
  Cholesky.

## Self-documenting mathematics

- `///` prose stored beside executable source.
- Explicit render blocks (`\{ ... \}`), render lines (`\[ ... \]`), and the
  `=>` evaluated-result marker.
- Shared scope between ordinary statements and rendered calculations.
- `#{ expression }` interpolation in prose and raw LaTeX strings in render
  position.
- AST-to-LaTeX rendering with precedence-aware fractions, powers, roots,
  implicit products, Greek names, subscripts, vectors, matrices, calculus
  notation, and derivation steps.
- Whole-document assembly, generated `.tex` output, and native PDF export
  through `pdflatex`.
- Last-known-good preview retention while an edit is temporarily invalid.
- Executable assertions and equation chains, allowing a document build to fail
  when a mathematical claim is not established.

The format is specified in [`DOCUMENTER.md`](DOCUMENTER.md), with design
decisions recorded in [`decisions.md`](decisions.md).

## Command line and terminal REPL

- Run a `.scas` file, read a program from standard input, or start the REPL
  automatically when standard input is a terminal.
- Build executable source into TeX with `scas build FILE [OUTPUT.tex]`.
- Set decimal precision, transcendental precision, evaluation steps, recursion
  depth, exponent, shift, and parser-nesting limits from CLI flags.
- Nonzero process status and rendered source diagnostics for lexical, syntax,
  runtime, I/O, and resource-limit failures.
- Persistent REPL definitions, history, reset/load-style commands, precision
  control, multiline entry, and terminal-aware prompts.

## Language server

The `scas-lsp` binary communicates over standard LSP JSON-RPC and provides:

- live scanner and recovering-parser diagnostics;
- builtin and user-function completion;
- hover evaluation and symbol information;
- signature help;
- go to definition;
- references;
- symbol rename.

## Native desktop IDE

- Rust-native `eframe`/`egui` application with no embedded browser runtime.
- IDE layout with a custom title bar, activity rail, resizable Explorer,
  document/REPL/Graphing workspaces, live preview, bottom dock, and status bar.
- File creation, Quick Open, save/save-as, unsaved-change guards, project tree,
  compact tabs, and breadcrumbs.
- Syntax highlighting for language tokens, comments, documentation, strings,
  delimiters, operators, and render markers.
- Unicode-aware inline diagnostic underlines, Problems navigation, hover
  values, go to definition, references, and rename.
- Prefix completion for builtins and user functions plus snippets for language,
  matrix, render, and documented-calculation constructs.
- Absolute, relative, hybrid, or disabled physical line numbers.
- Optional LaTeX-style Greek shortcut expansion in editor, REPL, and graph
  inputs.
- Background execution and debounced analysis with stale-result protection.
- A persistent notebook-style REPL with live typeset input, typeset results,
  history, numeric evaluation shortcut, precision control, and inline plots.
- A graphing calculator with multiple functions, adaptive background sampling,
  pan and zoom, automatic bounds, roots and extrema, hover/pinned probes, and
  SVG export.
- A resizable document preview with structured fallback rendering and a system
  Computer Modern path using `latex`, `dvisvgm`, and `rsvg-convert`; zoomed
  previews rerasterize at an appropriate resolution.
- Output, Problems, References, and embedded PTY terminal tabs.
- TeX and PDF export.
- Twelve bundled light and dark developer themes and external TOML themes.
- Bundled JetBrains Mono plus user-loaded `.ttf`/`.otf` editor fonts.
- Cross-launch persistence for theme, custom theme/font paths, editor font
  size, line-number mode, and Greek-shortcut preference.

## Browser IDE

- An `eframe`/`egui` source editor with syntax highlighting, tabs for Document,
  REPL, and Graphing, Run actions, clickable Problems, and responsive panels.
- Shared Rust lexer, parser, evaluator, document builder, plotter, and
  structured-preview model compiled to WebAssembly.
- Evaluation, persistent REPL submissions, and graph sampling in a Web Worker
  with generation IDs and deterministic limits.
- MathJax-backed live document preview with last-good output and TeX copying.
- Persistent browser REPL with definitions, history, reset, and recovery after
  errors.
- Pannable and zoomable graphing with SVG download.
- `.scas` import/download, TeX download, drag-and-drop `.scas`/CSV/JSON data,
  browser virtual files, Starter and MIT examples, and URL-hash source sharing.
- Local draft recovery and static output suitable for GitHub Pages.

The static build is produced by [`scripts/build-web.sh`](../scripts/build-web.sh).
The configured public deployment still requires verification.

## Reliability and engineering quality

- Unicode-aware line, column, token-length, and half-open UTF-8 byte spans
  through scanning, parsing, diagnostics, editor selection, and LSP conversion.
- Lexical, syntax, runtime, and limit diagnostics with source excerpts.
- Recovering parsing for editor diagnostics and fail-fast execution for batch
  programs.
- Deterministic limits for source size, tokens, syntax nesting, evaluation
  steps, call depth, exponents, shifts, decimal scale and precision,
  transcendental precision, number size, and document preview work.
- Typed `Result` error propagation for normal user failures instead of panic
  recovery.
- Unit, integration, property, CLI, golden-fixture, web-runtime, and GUI tests.
- Executed passing and failing `.scas` fixtures with checked output and
  diagnostics.
- Scanner and parser fuzz targets with seed corpora.
- Criterion baselines for scanning, parsing, exact arithmetic, symbolic
  expansion, and interpreted calls.
- Seeded differential comparison with SymPy: 100 generated cases across
  expansion, differentiation, polynomial GCD, and resultants.
- Fixed-workload SCAS/SymPy benchmarks for polynomial GCD, resultants, an 8×8
  determinant, and 50-digit gamma evaluation.
- CI workflows for formatting, Clippy, tests, MSRV, fuzzing, documentation,
  Pages, and versioned release artifacts with checksums and provenance.

See [`proof-of-correctness.md`](proof-of-correctness.md),
[`performance.md`](performance.md), and
[`sympy-benchmarks.md`](sympy-benchmarks.md) for the evidence and its limits.

## Architecture at a glance

```text
SCAS source → scanner → parser → AST → interpreter → exact/symbolic engines
                                      │                    │
                                      └→ document builder → LaTeX/structured math
                                                               │
                            CLI / REPL / LSP / native IDE / browser IDE + Worker
```

The crate separates lexer, parser, AST, numeric types, interpreter,
polynomials, plotting, document construction, LaTeX, intelligence services,
runtime I/O, native UI, web UI, and web-worker protocol. The library is built
as both `rlib` and `cdylib`, allowing native binaries and Wasm to share the
same core.

## Technical scope demonstrated

SCAS provides concrete implementation evidence for:

- language design, grammar definition, scanning, recursive-descent parsing,
  AST modeling, lexical scope, closures, and interpreter construction;
- exact arithmetic, arbitrary-precision real and complex numerical methods,
  polynomial algorithms, symbolic calculus, and linear algebra;
- conservative symbolic rewriting, explicit assumptions, and executable
  correctness checks;
- native desktop application architecture, background workers, syntax-aware
  editing, graph interaction, terminal integration, and preference storage;
- Wasm integration, Web Workers, virtual I/O, browser persistence, and static
  deployment;
- LSP protocol implementation and shared source-intelligence services;
- defensive diagnostics, deterministic resource governance, testing, fuzzing,
  benchmarking, CI, and release automation.

## Resume-ready summary

- Built a computer algebra scripting language in Rust from a Unicode-aware
  scanner and recursive-descent parser through a lexically scoped interpreter,
  CLI, REPL, LSP server, native IDE, and Wasm browser IDE.
- Implemented exact integer/rational arithmetic and arbitrary-precision real
  and complex mathematics, including principal-branch elementary and special
  functions with explicit numerical approximation.
- Developed a symbolic engine for conservative simplification, assumptions,
  polynomial manipulation and factoring, equation solving, differentiation,
  integration, series, limits, ODEs, rewrite rules, and exact/numeric linear
  algebra.
- Designed executable mathematical documents combining prose, source,
  evaluated derivations, checked claims, AST-to-LaTeX rendering, live preview,
  and TeX/PDF export.
- Shipped a native egui IDE with project navigation, diagnostics, completion,
  REPL, graphing, terminal, high-resolution Computer Modern preview, persistent
  editor preferences, and a twelve-theme system.
- Hardened the toolchain with typed diagnostics, resource limits, property and
  differential tests, fuzz targets, benchmarks, CI, and release provenance.

## Concise project-section version

> **SCAS — symbolic computer algebra language and IDE (Rust).** Built a
> Unicode-aware scripting language with exact rational and arbitrary-precision
> real/complex arithmetic, domain-aware symbolic algebra, polynomial solving,
> calculus, and matrix computation. Shipped CLI, REPL, LSP, native egui IDE,
> and Wasm browser IDE surfaces with diagnostics, completion, graphing, live
> mathematical preview, checked executable documents, deterministic limits,
> fuzzing, differential tests, and benchmarks.

## Current boundaries

- Symbolic integration, limits, sums, products, ODEs, and rewrite matching use
  bounded rule families. Unsupported cases remain unevaluated or return a clear
  error; SCAS does not implement a general Risch algorithm, general symbolic
  summation, or unrestricted ODE solving.
- `solve` handles supported univariate polynomial and invertible elementary
  forms. It does not yet solve general systems, symbolic inequalities, or
  cubic/quartic equations in full radical form.
- Periodic solutions are exposed separately through `solve_periodic` for
  affine sine, cosine, and tangent equations.
- Factoring is strongest for univariate integer/rational polynomials. General
  multivariate factorization, Gröbner bases, and algebraic-number
  normalization are not implemented.
- Partial fractions cover repeated rational linear factors and powers of one
  remaining irreducible quadratic factor; several-quadratic decompositions and
  fully general rational integration remain open.
- Assumptions detect direct contradictions and support useful sign/domain
  facts, but do not yet provide a complete derived-fact lattice, scoped
  assumptions, or branch-cut-aware complex reasoning.
- Numeric eigenpairs and SVD use bounded matrix sizes and machine-precision
  internal paths for the larger cases; defective eigensystems and
  high-precision numeric decompositions remain limited.
- Complex special functions can report convergence failure at difficult
  arguments or requested precision rather than returning an unverified value.
- The browser IDE omits the native PTY terminal and native PDF toolchain, and
  its configured public Pages deployment has not yet been verified.
