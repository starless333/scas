# SCAS project capabilities

SCAS is a from-scratch, dynamically typed programming language and symbolic
computer algebra system written in Rust. It combines an exact and
arbitrary-precision math engine, a scanner/parser/interpreter toolchain, a
self-documenting mathematical source format, a command-line workflow, and a
native desktop IDE.

This document is both a current capability inventory and a portfolio-oriented
overview. It describes implemented behavior; future ideas are intentionally
excluded.

## Executive overview

The project demonstrates end-to-end language engineering rather than a thin
wrapper around an existing evaluator. Source text is tokenized with Unicode-
aware spans, parsed into an AST, evaluated in lexical environments, rendered
as precise diagnostics, and optionally transformed into a live LaTeX-backed
document. The numerical layer preserves exact integers and rationals, supports
arbitrary-precision decimal and complex computation, and crosses into numeric
approximation only when the program requests it. The symbolic layer performs
safe simplification, polynomial algorithms, equation solving, calculus, and
exact linear algebra.

The same core powers three product surfaces:

- a batch CLI for files and pipelines;
- a persistent interactive REPL;
- a native `eframe`/`egui` IDE with editing, diagnostics, execution, and live
  mathematical preview in one workspace.

## Language and runtime

- Dynamically typed values: arbitrary-size integers, reduced rationals,
  arbitrary-precision decimals, arbitrary-precision complex numbers, symbolic
  expressions, strings, booleans, `nil`, arrays, matrices, and functions.
- Variables, declarations, assignment, compound assignment, printing,
  expressions, block scope, `if`/`else if`/`else`, `while`, C-style `for`,
  `break`, `continue`, and `return`.
- Expression-bodied mathematical functions and block-bodied functions.
- First-class functions and lexical closures backed by linked environments.
- Unicode identifiers, including built-in aliases `π`, `τ`, `φ`, and `Γ`.
- Implicit multiplication such as `2x`, `3(x + 1)`, and `(x + 1)y` with
  explicit precedence rules.
- Zero-based array and matrix indexing, including `A[row, column]` syntax.
- Structural array equality, string concatenation, array concatenation,
  short-circuit logical operators, integer bitwise operators, shifts, and
  exact numeric comparison across representations.
- Reusable Rust embedding APIs with caller-owned output, configurable
  precision, and persistent-interpreter access.

## Numeric computation

- Exact arbitrary-size integer and rational arithmetic.
- Arbitrary-precision decimal arithmetic with a configurable significant-digit
  target and guard digits where algorithms require them.
- Arbitrary-precision complex arithmetic for elementary functions and powers,
  using principal branches instead of silently rejecting non-real results.
- Exact symbolic constants `pi`, `e`, `tau`, and `phi`.
- Roots, exponential and logarithmic functions, circular and hyperbolic
  trigonometry, inverse functions, angle conversion, rounding, magnitude,
  sign, min/max, hypot, GCD, and LCM.
- Arbitrary-precision real `gamma`, `lgamma`, `erf`, and `erfc` implemented
  with series, continued fractions, recurrence/reflection, and asymptotic
  methods.
- Explicit `evalf` boundary between exact symbolic work and requested numeric
  approximation.
- Clear errors for poles, unsupported complex special functions, invalid
  dimensions, non-finite results, and exhausted resource budgets.

The complete documentation suite begins at [`index.html`](index.html). Its
callable reference is [`builtins.html`](builtins.html), with branch and
precision detail also available in [`math-functions.md`](math-functions.md).

## Symbolic algebra and equation solving

- Immutable reference-counted symbolic expression trees.
- Conservative identity simplification that does not assume an unknown symbol
  is real, positive, or nonzero.
- Session-local assumptions for real, integer, nonzero, nonnegative, positive,
  and negative variables, including contradiction detection.
- Canonical sparse multivariate polynomial representation.
- Collection, expansion, content extraction, common-monomial factoring,
  square/cube identities, square-free decomposition, rational-root factoring,
  partial fractions, and Horner form.
- Exact substitution and rational-expression manipulation.
- Exact polynomial invariants and Euclidean tools: degree, coefficients,
  resultant, discriminant, polynomial GCD, quotient, and remainder.
- Equation solving with exact rational and quadratic roots, complex quadratic
  roots, numerical complex roots for remaining univariate polynomial factors
  through degree 64, and inversion of supported affine elementary functions.

## Calculus and linear algebra

- Symbolic differentiation for algebraic powers, roots, exponential and
  logarithmic functions, trigonometric and hyperbolic families, inverse
  functions, error functions, and assumption-qualified absolute values.
- Symbolic integration for polynomial powers, affine substitutions,
  reciprocals, roots, exponential/trigonometric/hyperbolic families,
  logarithms, inverse functions, and error functions.
- Explicit unevaluated integrals when no safe rule is known.
- Gradients, Jacobians, Hessians, and definite integrals.
- Exact and symbolic matrix transpose, trace, determinant, rank, RREF,
  inverse, matrix × matrix multiplication, matrix × vector multiplication,
  and unique linear-system solving.
- Postfix transpose notation (`A'`) with the same behavior as `transpose(A)`.
- Vector dot products, cross products, and Euclidean norms.
- Both `matmul(A, B)` / `matmul(A, v)` and natural `A * B` / `A * v`
  multiplication syntax with dimension validation.
- Square-matrix integer powers computed by exponentiation by squaring,
  including identity matrices for exponent zero and exact inverse powers.
- Structured derivation variants that preserve row operations and explanatory
  steps for rendered mathematical work.

## Self-documenting mathematics

- `///` prose embedded directly beside executable source.
- Explicit render blocks (`\{ ... \}`), render lines (`\[ ... \]`), and the
  `=>` evaluated-result marker.
- Shared execution scope between normal code and rendered work.
- Total AST-to-LaTeX expression rendering with precedence-aware fractions,
  powers, roots, implicit products, Greek names, subscripts, vectors, matrices,
  calculus notation, and raw LaTeX escapes.
- Whole-document assembly, prose interpolation, generated TeX export, and PDF
  export through `pdflatex`.
- Step-by-step display of row reduction and other linear-algebra derivations.
- Last-known-good preview retention while a source edit is temporarily invalid.

## Native desktop IDE

- Rust-native `eframe`/`egui` application; no browser runtime is required.
- Familiar IDE topology with activity rail, resizable Explorer, source editor,
  live document preview, OUTPUT/PROBLEMS dock, and status bar.
- Syntax highlighting for language tokens, comments, documentation, strings,
  delimiters, and render markers.
- Unicode-aware diagnostic underlines, source locations, and hover detail.
- Line wrapping that responds to the editor's available width when the
  resizable renderer/preview pane narrows the code surface; physical line
  numbers remain stable across visual continuation rows.
- Prefix completion for language constructs and every builtin, cursor-anchored
  suggestions, and reusable snippets for functions, control flow, matrices,
  render blocks, and documented calculations.
- Automatic Greek shortcut expansion with a complete common alphabet and
  variant set.
- Background execution and bounded, debounced preview analysis so stale work
  cannot accumulate.
- Workspace tabs for source documents, a persistent interactive REPL, and a
  reserved graphing-calculator canvas.
- Notebook-style REPL history with session-persistent definitions, live
  AST-to-LaTeX rendering while expressions are entered, and typeset results.
- System LaTeX/Computer Modern rendering through serialized `latex` and
  `dvipng` workers, with a native structured renderer as a fast fallback.
- TeX/PDF export, saving, project browsing, configurable fonts and font size,
  line-number modes, and keyboard shortcuts.
- Twelve bundled light and dark developer themes plus user-loaded TOML themes.

## Reliability, security, and engineering quality

- Unicode-aware line, column, token-length, and UTF-8 byte spans throughout
  scanning, parsing, diagnostics, and editor highlighting.
- Phase-specific lexical, syntax, runtime, and limit diagnostics with source
  excerpts and exact underlines.
- Deterministic limits for source size, token count, syntax nesting, evaluation
  steps, call depth, exponents, shifts, decimal precision/scale,
  transcendental precision, and number size.
- No panic-based user error path for normal language failures; errors propagate
  through typed Rust results.
- Unit, integration, property-style, CLI, golden-file, and GUI tests.
- Executable passing/failing `.scas` fixtures with checked stdout/stderr.
- Scanner and parser fuzz targets.
- Benchmarks for parser, execution, symbolic, and rendering baselines.
- CI-oriented formatting, Clippy, all-target/all-feature tests, MSRV coverage,
  release archives, checksums, and build-provenance attestations.
- Clear subsystem separation across lexer, parser, AST, interpreter, numeric
  backend, polynomial engine, document builder, LaTeX renderer, REPL, CLI, and
  GUI.

## Technical scope demonstrated

SCAS provides concrete evidence of capability in:

- programming-language design, grammar definition, parsing, AST modeling, and
  interpreter construction;
- exact arithmetic, arbitrary-precision numerical methods, complex analysis,
  computer algebra, polynomial algorithms, calculus, and linear algebra;
- branch-safe symbolic rewriting and domain-aware correctness;
- native desktop application architecture, responsive pane behavior,
  background workers, syntax-aware editing, and theme systems;
- diagnostics UX, resource governance, defensive error handling, and embeddable
  API design;
- test strategy across deterministic examples, randomized identities,
  independent high-precision reference vectors, fuzzing, and benchmarks;
- technical writing spanning a language reference, architecture notes,
  decisions, error behavior, performance, themes, and an exhaustive function
  reference.

## Resume-ready summary

Use or adapt these bullets to fit the role and available space:

- Built a computer algebra scripting language in Rust from scanner and
  recursive-descent parser through a lexically scoped interpreter, REPL, CLI,
  embeddable API, and native desktop IDE.
- Implemented exact integer/rational arithmetic and arbitrary-precision real
  and complex mathematics, including principal-branch elementary functions,
  real special functions, deterministic resource limits, and precision-aware
  error handling.
- Developed a symbolic engine for safe simplification, assumptions, sparse
  multivariate polynomials, factoring, partial fractions, equation solving,
  differentiation, integration, and exact linear algebra.
- Designed executable mathematical documents that combine prose, source,
  evaluated derivations, AST-to-LaTeX rendering, live Computer Modern preview,
  and TeX/PDF export.
- Shipped a Rust-native IDE with project navigation, syntax highlighting,
  Unicode diagnostics, completions, responsive code wrapping, background
  analysis, resizable preview/output panes, and a configurable 12-theme system.
- Hardened the toolchain with typed phase-specific diagnostics, bounded
  execution, golden integration fixtures, property tests, high-precision
  reference vectors, fuzz targets, benchmarks, CI checks, and release
  provenance.

## Concise project-section version

> **SCAS — symbolic computer algebra language and IDE (Rust).** Built a
> Unicode-aware language toolchain with exact rational and arbitrary-precision
> real/complex arithmetic, symbolic algebra, polynomial solving, calculus, and
> exact matrix/vector operations. Created a native egui IDE with syntax-aware
> editing, diagnostics, completion, background execution, live LaTeX preview,
> and TeX/PDF export. Added deterministic resource limits, fuzz targets,
> property/integration tests, benchmarks, CI checks, and configurable theming.

## Current boundaries

- Complex extensions of `gamma`, `lgamma`, `erf`, and `erfc` are not yet
  implemented.
- Periodic integer-parameter solution families are not represented; supported
  trigonometric solving returns distinct principal solutions.
- Partial fractions intentionally target proper rational functions with
  distinct rational linear factors.
- Symbolic integration is rule-based and returns an unevaluated integral when
  no safe rule is available.
- The native IDE is described as an early preview, while the language and
  evaluator remain usable independently through the CLI and Rust API.
