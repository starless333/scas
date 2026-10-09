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

- [ ] **IO trait/abstraction.** Native: real files, stdin, command-line args. Browser: in-memory virtual filesystem, prompt-based `input()`, drag-and-drop CSV/JSON import.
- [ ] **Modules and `import`**, plus a small standard library written in SCAS itself (also a good way to test the language).
- [ ] **Maps/dictionaries.**
- [ ] **String formatting/interpolation.**
- [ ] **Better error recovery and runtime introspection** (e.g. `type_of`, `help(name)` surfacing the builtin reference).

## 4. CAS depth (go deep on a few things)

"Everything other CAS have" is a decades-long target. Prioritize what people recognize, roughly in this order:

1. [ ] **Complete limit and series coverage.** `limit` (including one-sided calls), `series`, sums, and products are implemented; limits at infinity, a principled L'Hopital path, and Laurent-series coverage still need review.
2. [ ] **Paired, ordered spectral decomposition and SVD.** `eigenvalues`, `eigenvectors`, `jordan_form`, LU, QR, and Cholesky are implemented; one stable paired eigen API and SVD remain open.
3. [ ] **Integer polynomial factoring** (e.g. Zassenhaus with Hensel lifting) to go beyond rational-root factoring.
4. [ ] **Periodic trigonometric solution families** with an integer parameter (`n in Z`), closing a listed boundary.
5. [ ] **Summation and products** (closed forms for polynomial, geometric, and simple hypergeometric sums) and **basic ODEs** (separable, first-order linear, constant-coefficient).
6. [ ] **A user-writable rewrite-rule system** (pattern matching on expressions, in SCAS itself).
7. [ ] **Broader partial fractions** (repeated and irreducible quadratic factors) and **stronger integration** (integration by parts, substitution search, rational-function integration; Risch-style later).
8. [ ] **Complex extensions** of `gamma`, `lgamma`, `erf`, `erfc`.

## 5. Features only SCAS can offer

These differentiate SCAS from black-box CAS tools.

- [ ] **Step-by-step output beyond linear algebra**: derivative rules, integration by parts and substitution, limit steps, equation-solving steps.
- [ ] **`assert` / `verify` primitive** so documents check their own claims, e.g. `assert(matmul(Q, Lambda, inverse(Q)) == B)`, and fail the build loudly if wrong.
- [ ] **Equation-chain checking**: each step of a derivation is verified equivalent to the previous one.

## 6. Proof of correctness

- [ ] **Differential testing against SymPy**: generate random expressions, compare results, and publish the pass rate.
- [ ] **Benchmark page** comparing SCAS and SymPy on a fixed set (polynomial GCD, large determinant, high-precision `gamma`, resultants). Publish honest numbers.
- [ ] **Keep the existing strengths visible**: fuzz targets, golden fixtures, reference vectors, deterministic limits, CI with provenance.
- [x] **Update `project-capabilities.md`** to match the current state (IDE, linear algebra, plotter, browser preview) and remove stale boundaries as they are closed.

## 7. Presentation

- [ ] README with a GIF/screenshot at the top and the live playground link.
- [x] Short architecture diagram (below).
- [ ] A blog post on one hard problem, e.g. keeping simplification sound without assuming symbols are real, or designing the exact/approximate boundary.
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

- Complex extensions of `gamma`, `lgamma`, `erf`, and `erfc` are not implemented.
- Periodic integer-parameter solution families are not represented; trig solving returns distinct principal solutions.
- Partial fractions target proper rational functions with distinct rational linear factors.
- Symbolic integration is rule-based and returns an unevaluated integral when no safe rule exists.
- Basic limits, series, sums, and products are implemented. Comprehensive
  infinity/one-sided limit coverage, Laurent series, and ODEs remain open.
