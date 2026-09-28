# Changelog

SCAS follows Semantic Versioning. User-visible changes accumulate under
`Unreleased`; releases move those entries beneath a dated version heading.

## Unreleased

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
