# SCAS Roadmap and Known Issues

SCAS is a from-scratch, dynamically typed programming language and symbolic computer algebra system written in Rust. It has exact and arbitrary-precision numerics, a symbolic engine, exact linear algebra, a native egui IDE with live LaTeX preview, a Desmos-style plotter, and self-documenting mathematical source that exports to LaTeX/PDF.

This document covers what to build next, in priority order, and the rough edges found while reviewing the exported output of the MIT 18.06SC Session 22 worked solutions (`solution.scas` -> `solution.tex`).

> Status: the browser IDE now builds locally as a static Wasm site. GitHub Pages deployment is configured but has not been verified at a public URL. The browser UI shares the language core and math preview with the native IDE; native-only services remain separate.

---

## 1. Known quirks to fix

These come from reviewing `solution.scas` and the exported `solution.tex`. All of the exported math was correct; these are presentation and design issues.

Resolved here: row-operation scalars are simplified, unit coefficients are omitted, and grouped derivations no longer nest `aligned` inside `align*`. Eigenvector/value pairing and a user-selectable eigenvalue order remain open.

### Output and rendering

| # | Issue | Where it shows up | Suggested fix |
|---|-------|-------------------|---------------|
| 1 | **Eigenvector output isn't paired with eigenvalues.** `solve` returns lambda = 2, 4, but `eigenvectors(A)` lists the lambda = 4 vector (2,1) first and the lambda = 2 vector (0,1) second, with no labels. | Problem 22.1 | Return and render pairs, e.g. `lambda=4: (2,1)^T`, `lambda=2: (0,1)^T`. Use one consistent ordering (ascending by eigenvalue, or by multiplicity) across `solve` and `eigenvectors`. |
| 2 | **Row-operation labels aren't simplified.** `R2 <- 1/(-13/9) R2` should read `R2 <- -(9/13) R2`. | `inverse_steps(Q)` | Simplify the scalar before rendering the label. |
| 3 | **Redundant parentheses around unit multipliers.** `R2 <- R2 - (1)R1` | `inverse_steps(S)`, `inverse_steps(A)` | Omit parentheses (and the coefficient) when the multiplier is 1; render `R2 <- R2 - R1`. |
| 4 | **Nested `aligned` inside `align*`** for grouped render blocks (e.g. the `trace`/`det_steps` block). It compiles, but spacing is uneven and the structure is fragile. | Problem 22.2, characteristic data | Emit a single flat `align*`/`aligned` per block, or use `gathered` for independent results. |
| 5 | **Eigenvalue ordering differs from textbook convention.** `solve` returns ascending order (2, 4), while the worked solution presents 4 then 2. | Problem 22.1 | Let the user control ordering, or document the default. |

### Language design

| # | Issue | Suggested fix |
|---|-------|---------------|
| 6 | **Two statement-terminator conventions.** `expr =>` outside a block and `expr =>;` inside `\{ ... \}` are easy to mix up. | Force =>; or remove ; necessity in the language. |
| 7 | **Matrix/vector inputs are hand-typed rather than derived.** `Q`, `Lambda`, `P`, and `D_inverse` in Problem 22.2 are entered by hand, so the document displays results rather than deriving them. | Use the implemented spectral and limit primitives in the example where they can replace hard-coded values; add a paired decomposition API for the remaining gap. |
| 8 | **Claims in prose are unchecked.** The general diagonalizing matrix `S'`, `lim Lambda^k`, and `lim B^k` appear only as raw LaTeX. | Add `assert`/`verify` (see section 4) so a wrong claim fails the build. |

---

## 2. WASM playground (highest priority)

The biggest gain is making the project easy to see. Most recruiters will not clone and build a Rust repo.

- [x] **Build a real egui browser IDE** with `eframe`'s web target: editor, output/problems, live structured preview, and graphing. The browser UI is a separate platform-specific `WebApp`, not the full native `ScasApp` port.
- [x] **Audit the bignum backend.** The core uses pure-Rust arbitrary-precision crates; `cargo check --lib --target wasm32-unknown-unknown` passes.
- [x] **Replace the system LaTeX pipeline in the browser** with the shared native structured renderer. TeX remains downloadable.
- [x] **Run evaluation in a Web Worker** with generation IDs and deterministic limits. Worker termination/cancellation of an already-running job remains open.
- [x] **Shareable links** that encode the program in the URL hash.
- [x] **Example gallery** with one-click Starter and MIT 18.06 documents.
- [ ] **Deploy** to GitHub Pages or similar, verify the public URL, and put the link at the top of the repo. The Pages workflow is in place.

## 3. IO and a more dynamic language

Design IO as an abstraction layer so the same code runs natively and in wasm.

- [x] **IO trait/abstraction.** `read_text`, `write_text`, `input`, and `args` share a host interface. Native uses files, stdin, and process arguments; the browser uses a virtual filesystem, prompts, and drag-and-drop `.scas`, CSV, and JSON uploads. `read_csv` and `read_json` convert uploaded data into SCAS arrays and maps.
- [x] **Modules and `import`**, plus a small standard library written in SCAS itself (`stdlib/core.scas`). Import paths are explicit and the browser bundles the standard library in its virtual filesystem.
- [x] **Maps/dictionaries.** First-class immutable maps support scalar keys, indexing, `put`, `keys`, and `values`.
- [x] **String formatting/interpolation.** `format()` replaces `{}` fields and supports escaped braces.
- [ ] **Better error recovery and runtime introspection.** `type_of` and `help(name)` are implemented; richer error recovery and comprehensive builtin help remain open.

## 4. CAS depth (go deep on a few things)

"Everything other CAS have" is a decades-long target. Prioritize what people recognize, roughly in this order:

1. [x] **Practical limit and series coverage.** Rational-function limits use exact polynomial valuations at finite rational points, including poles of any multiplicity, and exact degree comparisons at both infinities. Directional affine absolute-value expressions and bounded L'Hopital cases are supported. `series` returns Taylor and Laurent terms through order 64 for rational polynomial quotients, with an optional explicit `O((x-a)^(n+1))` term via `series(f,x,a,n,"O")`. General branch-sensitive limits outside these families remain future work.
2. [x] **Paired, ordered spectral decomposition and SVD.** `eigenpairs` pairs exact values and vectors through 2×2 and for triangular matrices with available eigenvectors. Larger real symmetric matrices use numeric Jacobi; general numeric matrices through 6×6 use residual-checked complex eigenpairs. `natural`, `ascending`, and `descending` ordering preserve pairings, with numeric ordering restricted to real eigenvalues. `svd` returns `[U, Sigma, Vt]` for real or complex numeric rectangular matrices; 1×1 and 2×2 real square cases retain exact arithmetic. Larger defective eigensystems and high-precision numeric SVD remain future work.
3. [x] **Integer polynomial factoring for small degrees.** Rational roots, square-free parts, exact quadratic and monic cubic divisor searches, and approximate root-subset reconstruction cover many degree 4–10 integer polynomials, including nonmonic quartic products. Every proposed factor is verified by exact division over the rationals. Higher degrees and ill-conditioned cases may remain unfactored; general Zassenhaus/Hensel factoring is future work.
4. [x] **Periodic trigonometric solution families**: `solve_periodic(expr, x, n)` returns sine, cosine, and tangent families for affine arguments with `n` explicitly in `Z`.
5. [ ] **Summation and products** (closed forms for polynomial, geometric, and simple hypergeometric sums) and **basic ODEs** (separable, first-order linear, constant-coefficient). Polynomial sums now cover powers through eight; geometric sums, separable implicit solutions, first-order linear ODEs, and constant-coefficient cases are implemented. Hypergeometric sums and broader ODE families remain open.
6. [ ] **A user-writable rewrite-rule system**: `rewrite(expr, pattern, replacement)` performs one finite top-down pass with repeated wildcard bindings checked for consistency. Persistent rule collections and conditional rules remain open.
7. [ ] **Broader partial fractions**: repeated rational linear factors and one remaining irreducible quadratic factor now decompose exactly. Integration by parts and several substitution and rational cases are implemented; repeated quadratic factors, wider substitution search, and general rational integration remain open.
8. [x] **Complex extensions** of `gamma`, `lgamma`, `erf`, `erfc`. Complex `gamma` uses reflection, recurrence, and a Stirling expansion; `lgamma` takes the principal logarithm. Complex `erf` and `erfc` use a series near the origin and an asymptotic expansion for larger arguments, reporting a convergence error when the requested precision cannot be met.

## 5. Features only SCAS can offer

These differentiate SCAS from black-box CAS tools.

- [x] **Step-by-step output beyond linear algebra**: document results now show derivative rules, integration by parts and substitution setup with derivative checks, direct limit substitution, and factoring before equation roots. Deeper recursive traces remain open.
- [x] **`assert` / `verify` primitive** so documents check their own claims, e.g. `assert(matmul(Q, Lambda, inverse(Q)) == B)`, and fail the build loudly if wrong.
- [x] **Equation-chain checking**: `verify_chain([step1, step2, ...])` checks every adjacent pair by exact polynomial arithmetic or structural equality and fails at the first unproved link.

## 6. Proof of correctness

- [ ] **Differential testing against SymPy**: generate random expressions, compare results, and publish the pass rate.
- [ ] **Benchmark page** comparing SCAS and SymPy on a fixed set (polynomial GCD, large determinant, high-precision `gamma`, resultants). Publish honest numbers.
- [ ] **Keep the existing strengths visible**: fuzz targets, golden fixtures, reference vectors, deterministic limits, CI with provenance.
- [x] **Update `project-capabilities.md`** to match the current state (IDE, linear algebra, plotter, browser preview) and remove stale boundaries as they are closed.

## 7. Presentation

- [x] README with a browser IDE screenshot at the top and the configured playground URL. Public deployment still needs verification.
- [x] Short architecture diagram (below).
- [x] [Design note on the exact/approximate boundary](exact-approximate-boundary.md).
- [x] Language server (LSP) for editor support outside the IDE; documenter
  diagnostic/completion parity with the GUI remains open.

```text
SCAS source → lexer → parser → AST → interpreter → exact numeric + symbolic engines
                                    │                    │
                                    └→ document builder → LaTeX + structured math
                                                            │
                                      CLI / REPL ─ native IDE ─ browser IDE (Wasm + Worker)
```

---

## Suggested order of work

1. Fix the quirks in section 1 (quick wins, especially #1-#3).
2. WASM playground with examples and shareable links (section 2).
3. Limits and series, then eigen-decomposition (section 4, items 1-2).
4. `assert`/`verify` and calculus step output (section 5).
5. IO abstraction, modules, maps, string formatting (section 3).
6. SymPy comparison and benchmarks (section 6).
7. Integer factoring, ODEs, summation, rewrite rules as stretch goals.

## Current boundaries (from the capability inventory)

- Complex `gamma`, `lgamma`, `erf`, and `erfc` have numerical paths; difficult arguments can still produce a convergence error.
- `solve` returns principal trigonometric solutions; `solve_periodic` returns integer-parameter families for affine sine, cosine, and tangent equations.
- Partial fractions handle repeated rational linear factors and one remaining irreducible quadratic factor.
- Symbolic integration is rule-based and returns an unevaluated integral when no safe rule exists.
- Basic limits, series, sums, and products are implemented. General branch-sensitive
  limits, big-O series, and broader ODEs remain open.
