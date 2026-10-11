# Changelog

SCAS follows Semantic Versioning. User-visible changes accumulate under
`Unreleased`; releases move those entries beneath a dated version heading.

## Unreleased

- The native GUI now restores the selected theme, custom theme and font files,
  editor font size, line-number mode, and Greek-shortcut setting across launches.
- Made the GUI graphing canvas dynamic like Desmos: pan and zoom re-sample
  curves over a padded window that always covers the visible region, dots
  mark zeros and local extrema of every curve, and clicking the canvas pins
  a snapping crosshair that reports the exact x/y of the nearest point.
- Made the GUI REPL LaTeX-first: typed input is live-typeset at display size
  with the raw source demoted to a faint echo underneath.
- Added general constraints via `assume(constraint)`: comparisons such as
  `x == 3`, `x > 0 and x < 10`, and `x != 0` are recorded as facts,
  equalities substitute into later expressions, and contradictions are
  rejected; `forget_assumptions()` clears them. The named
  `assume_real`/`assume_integer`/sign helpers remain and are mirrored by the
  recorded facts.
- Added function plotting: the REPL `:plot` command draws one or more
  expressions on a TI-style ASCII grid or redirects to an SVG/text file, and
  the GUI gained a full graphing-calculator tab (pan, zoom, auto-fit, SVG
  export) plus inline REPL plots, command history, and Tab completion. Plots
  evaluate through the live session, support plotted definitions such as
  `g(t) = 2t + 1`, and restore any displaced variable bindings.
- Added `A'` postfix matrix-transpose notation and matrix multiplication for
  matrix × matrix and matrix × column-vector operands.
- Added square-matrix integer powers using binary exponentiation, including
  identity and exact inverse behavior for zero and negative exponents.
- Added native IDE tabs for a live-typeset persistent REPL and a reserved
  graphing-calculator workspace.
- Replaced machine-precision math with arbitrary-precision real and complex
  elementary evaluation, exact symbolic constants, and explicit `evalf`.
- Added principal complex branches, first-class complex arithmetic, exact
  common-angle values, non-integral powers, and arbitrary-precision real
  `gamma`, `lgamma`, `erf`, and `erfc`.
- Added real/integer/positive/negative assumptions and made cancellation,
  roots, absolute values, and logarithm/exponential inverses domain-safe.
- Expanded differentiation and affine-substitution integration rules, added a
  dedicated transcendental resource budget, and documented numeric semantics.
- Fixed the chain rule for `tan`, including `D(tan(x)) = sec(x)^2`.
- Added reproducible property tests, fuzz targets, measured benchmarks, API
  documentation deployment, and attested cross-platform release automation.
- Added recovering parsing and brace-aware document interpolation discovery for
  IDE clients.
- Added line, column, and byte-range source spans to every statement and
  expression AST node.
- Added a dependency-free, precedence-aware source-expression LaTeX renderer.
- Expanded calculus with general-power and inverse/hyperbolic derivatives,
  constant-factor integration, and affine substitution.
- Added consistent diagnostics and execution resource limits.
- Added matrices, multi-indexing, `else if`, and the documented `for` grammar.
- Added collection, substitution, polynomial solving, differentiation, and
  integration with unevaluated fallbacks.
- Added file-oriented CLI options and completed the P2 documentation baseline.

## 0.1.0

- Initial exact-arithmetic language and symbolic algebra implementation.
