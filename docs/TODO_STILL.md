# TODO_STILL

A living checklist of everything still standing between SCAS and an
industry-grade computer algebra system. Items here are *not yet done* unless
marked otherwise. Companion documents:

- `docs/algebraic_identities.md` — the identity roadmap (unchecked entries are
  repeated here so this file is the single place to look for open work).
- `docs/project-capabilities.md` — what exists today, including the
  "Current boundaries" section.

Legend: `[ ]` todo, `[x]` done.

---

## 1. Algebraic identities and simplification depth

### 1.1 Calculus rewrites (open items from `algebraic_identities.md`)

- [x] Bounded integration by parts: polynomial × exponential and
      polynomial × trigonometric forms (e.g. `∫ x*e^x dx`, `∫ x*sin(x) dx`,
      iterated for `x^n` up to a rewrite budget). The tabular rule covers
      `exp`/`sin`/`cos`/`sinh`/`cosh` of linear arguments (the families
      closed under repeated integration) with an 8-degree expansion budget.
- [x] Integrate rational functions through partial-fraction decomposition
      (`apart` output feeds `integral`). Proper rational functions with
      distinct rational linear factors integrate term by term
      (`∫ (3x+1)/((x-1)(x+2)) dx = 4/3·ln(x-1) + 5/3·ln(x+2)`); `c/(a*x+b)`
      now integrates for any constant numerator. Improper fractions and
      repeated/irreducible factors still fall back (see 1.4).
- [x] Trigonometric forms in integrals: the classic tangent substitution
      `∫ c/(a*x^2 + b) dx = c*atan(x/sqrt(b/a))/(a*sqrt(b/a))` for positive
      rational `a`, `b` (`∫ 1/(1+x^2) dx = atan(x)`), and double-angle
      halves for `∫ sin(a*x+b)^2 dx` / `∫ cos(a*x+b)^2 dx`. General
      trigonometric substitution (Weierstrass-style) remains open — see 1.2.
- [x] Definite-integral rules: fundamental theorem of calculus on the current
      antiderivative set, symmetry exploitation (`∫_{-a}^{a}` of odd/even
      integrands), linearity over sums of limits. Integrands split over sums;
      odd integrands over symmetric intervals vanish even without a closed
      antiderivative; even ones fold to `2*∫_{0}^{a}`; integrals without a
      closed primitive stay unevaluated instead of receiving corrupted
      substitutions (see "Fixed in this pass").

### 1.2 Trigonometry and hyperbolic identities

- [ ] Sum-to-product and product-to-sum rewrites
      (`sin(a)cos(b) -> (sin(a+b) + sin(a-b))/2` and friends).
- [ ] Power-reduction and general multiple-angle formulas
      (`sin(x)^2 -> (1 - cos(2x))/2`, `cos(n*x)` expansion for integer `n`).
- [ ] Half-angle formulas with correct sign handling under assumptions.
- [ ] Hyperbolic identities: `cosh(x)^2 - sinh(x)^2 -> 1`,
      `1 - tanh(x)^2 -> sech(x)^2` (once `sech`/`csch` exist, see 2.6),
      double-angle and addition formulas.
- [ ] Inverse-function simplifications under assumptions
      (`sin(asin(x)) -> x` for `x in [-1, 1]`, `asin(sin(x))` only on the
      principal branch), `atan2`-aware forms.

### 1.3 Exponentials, logarithms, and radicals

- [ ] `log(a^n) -> n*log(a)` under `a > 0` (currently refused without
      assumptions; make the assumption path complete and tested).
- [ ] `log(a) + log(b) -> log(a*b)` and its inverse (`logcombine`/`logexpand`
      as explicit, separately controllable transforms).
- [ ] Radicals: denesting (`sqrt(3 + 2*sqrt(2)) -> 1 + sqrt(2)`), collecting
      `sqrt(a)*sqrt(b) -> sqrt(a*b)` only under valid sign assumptions,
      rationalizing denominators (`1/(1 + sqrt(2))`).
- [ ] Algebraic-number normalization: exact simplification of nested roots,
      minimal-polynomial based equality tests for radical expressions.
- [ ] Canonical complex-number handling of fractional powers (principal branch
      documented and tested at branch cuts).

### 1.4 Polynomial and rational-function identities

- [ ] General multivariate factorization (beyond the current special forms,
      common-content factoring, and univariate square-free decomposition).
- [ ] Rational-root theorem and quadratic/cubic/quartic closed-form root
      rewriting into exact radical form where the discriminant allows.
- [ ] Multivariate polynomial GCD and division (currently univariate over Q).
- [x] `together` (sum of fractions -> single fraction) as the inverse of
      `apart`, with full GCD cancellation under known-nonzero conditions.
      (Done: `together(expr)` builtin combines sums over a common
      denominator and cancels shared univariate content via
      `reduce_fraction`.)
- [x] Improper rational functions in `apart`: polynomial long division first,
      then partial fractions of the remainder. (Done: `apart((x^2 + 1) /
      (x + 1))` now yields `x - 1 + 2 / (x + 1)`, and `integral(x / (x + 1),
      x)` evaluates to `x - ln(x + 1)` through the same division.)
- [x] Partial fractions over repeated rational linear factors and powers of
      one irreducible quadratic factor. Multiple distinct irreducible
      quadratics remain a separate extension.
- [ ] Gröbner-basis style normalization for systems of polynomial relations
      (long term; foundation for `solve` of multivariate systems).

### 1.5 Boolean, comparison, and piecewise identities

- [ ] Full boolean algebra rewrites on *symbolic* predicates
      (`not (a < b) -> a >= b`, De Morgan on symbolic conditions).
- [ ] Absorption, idempotence, and normalization of comparison chains
      (`a < b < c` semantics and simplification).
- [ ] Piecewise expressions as first-class values (`piecewise(...)`) with
      simplification through case splitting; needed for `abs`, `floor`, `sign`
      composed with symbols.

### 1.6 Assumptions system upgrades

- [x] Detect contradictory assumptions (`assume_positive(x)` followed by
      `assume_negative(x)`) and report instead of silently proceeding. (Done
      and strengthened: bounds are compared pairwise so empty intervals like
      `assume(b > 2); assume(b < 1)` are rejected, equality-vs-bound checks
      follow the `bound op variable` convention (`assume(a > 2);
      assume(a == 3)` passes instead of falsely conflicting), and the named
      tools record facts so both directions cross-check
      (`assume(d == 0); assume_positive(d)` names `d` in the error).)
- [ ] Propagate derived facts: `x > 0 -> x != 0`, `x integer -> x real`,
      `x in (0, 1) -> x^2 < x`, etc. (Partial: `x > 0` now implies nonzero
      and nonnegative, `x == c` propagates the sign of `c`, and `assume_integer`
      implies real; conditional rewrites like `x^2 < x` on `(0, 1)` remain.)
- [x] Inequality assumptions on expressions, not just names (`assume(x + 1 > 0)`).
      (Done for linear expressions in one variable: the comparison is
      normalized to a plain bound, so `assume(x + 1 > 0)` records `x > -1`
      and `assume(2 * x < 6)` records `x < 3`, with full conflict checking.)
- [ ] `$Assumptions`-style global list plus `Refine`/`assuming` scoped blocks
      (Mathematica parity).
- [ ] Domain conditions returned alongside results: when a rewrite needs
      `x != 0`, surface the condition instead of only gating the rewrite.
- [ ] Complex-domain facts: `real(x)`, `imag(x)`, branch-cut-aware simplifier
      decisions for `log`, `sqrt`, `asin`, `atanh` over complex inputs.

### 1.7 Simplifier engineering

- [ ] AC (associative-commutative) canonicalization before local rewrites so
      output does not depend on rule order (guidance in
      `algebraic_identities.md`).
- [ ] Expression arena with hash-consing so structural equality and common
      subexpression reuse are cheap (guidance item 4).
- [ ] Rewrite budget / termination measure on every automatic rule.
- [ ] Complexity-function-driven `simplify` (leaf count or user-selectable)
      with `full_simplify` trying `expand`/`factor`/`trigsimp`/`powsimp`
      pipelines and keeping the smallest result.
- [ ] Randomized numeric substitution testing for identities (zero, negative,
      rational, high-precision decimal, complex boundary cases).

---

## 2. Symbolic math depth (big-CAS parity)

### 2.1 Solving

- [ ] Symbolic solution families: return parametric families instead of
      principal values only (`sin(x) = 0 -> x = k*pi` with `k integer` —
      currently noted in "Current boundaries").
- [ ] Solve polynomial equations of degree 3 and 4 exactly (radicals) with
      exact root simplification; degree ≥ 5 via `Root`-style algebraic-number
      objects (Mathematica `Root`, Maple `RootOf`).
- [ ] Systems of equations (`solve([x + y = 3, x - y = 1], [x, y])`),
      linear via the existing `solve_linear`, nonlinear via substitution and
      resultants (`resultant` already exists — wire it in).
- [ ] Inequality solving (`solve(x^2 - 1 > 0, x) -> x < -1 or x > 1`) with
      interval arithmetic as the substrate.
- [ ] Transcendental equations with monotonicity reasoning and numeric
      refinement of isolated roots.
- [ ] Absolute-value and floor/ceil equations via case splitting.
- [ ] `reduce`-style output that also reports conditions (e.g. `a*x = b`
      branches on `a = 0`).

### 2.2 Calculus

- [x] Finite symbolic limits (`limit(sin(x)/x, x, 0)`) through direct
      substitution and a bounded L'Hôpital heuristic. The optional
      `"left"`/`"right"` syntax is validated; infinity and branch-sensitive
      directional limits remain future extensions.
- [x] Taylor polynomials (`series(f, x, a, n)`) through repeated symbolic
      differentiation, exact point substitution, and factorial scaling.
      Laurent series and explicit big-O terms remain future extensions.
- [x] Multivariable differentiation (`gradient`, `jacobian`, `hessian`) and
      repeated symbolic integration (`multiple_integral(expr, [vars])`).
      Path-sensitive assumptions and general change-of-variables automation
      remain future extensions.
- [x] Exact finite `sum` and `product` over integer ranges, plus symbolic
      Faulhaber forms through eighth powers, geometric sums, and `k*r^k` for
      supported rational bases. General telescoping and additional known
      infinite/symbolic products remain future extensions.
- [x] `dsolve` for first-order linear ODEs and homogeneous
      constant-coefficient ODEs through second order; `dsolve_separable`
      returns supported implicit separable solutions. Broader separable forms
      and higher-order characteristic polynomials remain future extensions.
- [ ] Risch-style structure theorems eventually; until then grow the rule
      table (rational functions, erf/Gaussian forms, by-parts families).

### 2.3 Matrices and linear algebra

- [x] Characteristic polynomials and exact eigenvalues/eigenvectors for 1×1
      and 2×2 matrices, plus diagonal eigenvalues for larger triangular
      matrices. General higher-order numeric QR eigensolving remains open.
- [x] LU, QR, Cholesky decompositions with steps variants (matching the
      existing `*_steps` family: `det_steps`, `rref_steps`, …).
- [x] Null-space and column-space bases, 2×2 Jordan form, and exact diagonal
      matrix exponentials. Higher-dimensional Jordan chains and general
      matrix exponentials remain open.
- [ ] Structured matrices: symbolic entries with assumptions (e.g.
      det of a Vandermonde), sparse representation for large numeric cases.
- [x] Vector spaces: Gram–Schmidt, projections, change of basis.

### 2.4 Numeric computation

- [ ] Adaptive quadrature with error estimates for `definite_integral`
      (Gauss–Kronrod or adaptive Simpson), falling back to high-precision
      sampling only when rules fail.
- [ ] Root finding and optimization for numeric expressions (`find_root`,
      `minimize`/`maximize` with golden-section/Newton).
- [ ] Numeric ODE integration (RK45) for `dsolve` with initial conditions.
- [ ] Interpolation and least-squares fitting (`polyfit`, splines) —
      Desmos/Excel-grade data workflows.
- [ ] Compensated summation and condition-number reporting on ill-conditioned
      numeric linear algebra.

### 2.5 Special functions and constants

- [ ] Complex extensions of `gamma`, `lgamma`, `erf`, `erfc`
      ("Current boundaries" item 1).
- [ ] More special functions: factorial/binomial as builtins (currently
      reachable only via `gamma`), `beta`, incomplete gamma/beta, Riemann
      `zeta`, Bessel `J`/`Y`/`I`/`K`, hypergeometric `2F1`.
- [ ] Exact evaluation at special points (`gamma(1/2) = sqrt(pi)`,
      `zeta(2) = pi^2/6`) with assumption-guarded reflection formulas.
- [ ] Constants: exact continued-fraction recognition in `evalf` output
      (identify `3.14159…` as `pi` heuristically, like Wolfram|Alpha).

### 2.6 Language-level math affordances

- [ ] `sech`, `csch`, `coth` and their inverses to complete the hyperbolic
      family.
- [ ] Piecewise, `arg` (complex argument), `re`/`im`/`conjugate` as symbolic
      operations with simplification rules.
- [ ] Modular arithmetic (`modpow`, modular inverse) and number-theory
      builtins (primality, factorization) used by other CAS for exact work.

---

## 3. Graphing calculator (Desmos parity)

Done so far: live pan/zoom on cached samples, progressive fast→full
re-sampling, zeros/extrema dots, tap probe with crosshair, SVG export,
multi-series colors, auto y-fit.

- [ ] Parameter sliders: `a` in `a*x` binds to a draggable slider that
      re-samples at interactive rates (the signature Desmos feature).
- [ ] Editable function rows in the list (edit source in place; currently
      remove + re-add), with per-row color picker and show/hide toggle.
- [ ] Click-drag probe that reports coordinates continuously while dragging
      (currently click-to-pin only).
- [ ] Inequality plotting and shading (`y > sin(x)` region fill).
- [ ] Implicit plots (`x^2 + y^2 = 25`) and implicit inequalities.
- [ ] Parametric curves `(x(t), y(t))` and polar plots `r(θ)`.
- [ ] Tables of values synced to the plotted functions.
- [ ] Zoom-x-only / zoom-y-only modifiers (shift/alt + drag), square-axis
      lock, and a "home" key to reset the window.
- [ ] Touchpad two-finger scrolling pans the window (currently the wheel only
      zooms; drag is the sole pan).
- [ ] Keyboard accessibility: arrow keys pan, `+`/`-` zoom, tab focus order.
- [ ] Piecewise/discontinuity quality: asymptote detection drawn as dashed
      guides, hole markers for removable singularities.
- [ ] Graph state persistence (functions + window) across app restarts.
- [ ] Export PNG/PDF and "copy as image" in addition to SVG.
- [ ] Regression: once 2.4 lands, plot fitted models against data lists.

---

## 4. IDE / GUI bugs and improvements

### 4.1 Responsiveness and correctness

- [ ] `:plot` in the GUI REPL samples *synchronously on the UI thread*
      (`submit_repl_plot` → `plot::execute_plot_command`), so a heavy
      expression freezes the window. Run it through the background graph
      sampler (placeholder entry + async fill) while preserving the live
      session side effects of plot-command prefix statements.
- [ ] Long-running REPL evaluations have no interrupt: add a Stop button /
      Ctrl+C that trips the interpreter step limit path (the `Limits::max_steps`
      machinery is the hook).
- [ ] Document preview rebuilds the whole document and re-renders the full
      LaTeX body on each pause: cache unchanged fragments (keyed by source
      hash) so previews scale to long documents.
- [ ] `sample_expression` (`src/plot.rs`, used by the document renderer) is
      not cancellable; wire the same `should_cancel` mechanism used by the
      graph sampler into document preview plots.
- [ ] Error paths in `poll_graph_sampling` mark all functions stale when the
      worker channel disconnects, but never retry; surface a visible status
      and re-arm sampling.

### 4.2 Editor (features other IDEs treat as table stakes)

- [x] VS Code-style source diagnostics: severity-colored ranges, gutter markers,
      inline messages, and a Problems list that navigates to source.
- [x] Integrated PTY/VT terminal beside Output and Problems, with direct
      keystrokes, resizing, colors, alternate screen, and persistent scrollback.
- [ ] Complete terminal parity with dedicated emulators (selection/copy,
      extended mouse modes, hyperlinks, and less common escape sequences).
- [ ] Find / replace (with regex and case toggles).
- [ ] Go-to-line, bracket match highlighting, auto-closing brackets/quotes.
- [ ] Code folding for `\\{ … \\}` document blocks and `fn` bodies.
- [ ] Hover documentation for builtins (signature + one-line summary from
      `docs/math-functions.md`).
- [x] Signature help / parameter hints inside call parentheses.
- [ ] Quick fixes for common diagnostics (e.g. "add missing `;`").
- [ ] Multi-cursor editing and column selection.

### 4.3 REPL

- [ ] Multi-line input (shift+Enter continuation) for `fn`/`if` blocks.
- [ ] Persistent history across restarts, with history search.
- [ ] Click a result to copy it; copy-as-LaTeX for any rendered expression.
- [ ] Value inspection: click a matrix result to expand it into a grid view.

### 4.4 Shell and workflow

- [ ] Session/workspace persistence: open files, active tab, theme, window
      layout restored on launch.
- [ ] Recent-files menu; drag-and-drop files onto the window to open.
- [ ] Command palette (Ctrl+P / Ctrl+Shift+P) for actions and snippets.
- [ ] Split editor view; tabbed documents (currently one buffer at a time).
- [ ] Undo/redo for graph view changes and function-list edits.
- [ ] Status-bar indicators (cursor line/column, REPL state, sampling state).

### 4.5 Correctness polish noticed while working here

- [ ] The graph probe is discarded whenever a fresh sampling batch is applied
      (`apply_graph_batch`); Desmos keeps the tracked point attached to the
      curve across re-samples — re-snap the pinned point to the new samples
      instead of clearing it.
- [ ] Graph coverage hysteresis uses fixed 15%/30% padding; make it scale with
      sampling latency so very slow expressions get wider cached windows.
- [ ] The `graph_sample_count` field is informational only; surface it (or
      drop it) to avoid drift from the actual request count.
- [ ] Greek-shortcut expansion in the graph input mutates the text outside the
      widget's undo stack (the editor path adds undo entries manually); give
      the graph input the same undo treatment.

---

## 5. Engineering quality

- [ ] Benchmarks for the symbolic hot paths (extend `benches/baselines.rs`):
      simplify/expand/factor on large polynomials, evaluator throughput,
      plot sampling points/second.
- [ ] Property-based tests for identity rewrites (random expression trees →
      numeric substitution equality under generated assumptions).
- [ ] Fuzz the parser/lexer longer in CI (`fuzz/` exists) and add the GUI
      command parser (`parse_plot_command`) to the fuzz targets.
- [ ] Golden-file tests for the graph worker: superseded requests produce no
      stale results; content-generation mismatches are dropped.
- [ ] Deterministic output snapshots for SVG/LaTeX plot exports in CI.
- [ ] Benchmark-driven revisit of hash-consing the expression representation
      (item 1.7) before optimizing individual rules.
- [ ] CI matrix: stable + MSRV (1.79) + `--features gui` build on all three
      desktop platforms.

---

## Fixed in this pass (context for the lists above)

- Integration by parts now works for polynomial × {exp, sin, cos, sinh,
  cosh} of linear arguments via the tabular rule (e.g. `∫ x*e^x dx =
  x*e^x - e^x`, `∫ x^2*cos(x) dx = x^2*sin(x) + 2x*cos(x) - 2sin(x)`),
  with a degree budget keeping the expansion bounded.
- Rational functions integrate through partial fractions: `integral` feeds
  proper fractions with distinct rational linear factors through `apart` and
  integrates the `c/(linear)` terms (`∫ 1/x dx = ln(x)`, and three-factor
  denominators like `x^3 - x` work end to end).
- Trigonometric forms integrate: `∫ 1/(1+x^2) dx = atan(x)`,
  `∫ 1/(x^2+4) dx = atan(x/2)/2`, and `∫ sin(x)^2`/`∫ cos(x)^2` use the
  double-angle halves (`x/2 ∓ sin(2x)/4` scaled by the linear coefficient).
- Exact numeric arguments now fold perfect rational powers too
  (`sqrt(4) -> 2`, `cbrt(-8) -> -2`) in addition to the zero/one family.
- The rational-root enumeration (`root_candidates`) missed nonzero roots
  whenever the constant term was zero, so `apart` and `solve` both failed on
  polynomials like `x^3 - x`; the zero roots are now stripped before the
  divisor enumeration (`solve(x^3 - x, x)` finds `[-1, 0, 1]` instead of
  only `[0]`).
- Definite integrals: linearity over sums, odd/even symmetry on symmetric
  intervals, and FTC guarded to the closed antiderivative set. This also
  fixes a corruption bug where `definite_integral(f, x, a, b)` substituted
  limits into unevaluated primitives (`definite_integral(x^x, x, 0, 1)` used
  to produce garbage like `integral(1, 1) - integral(1, 0)`; it now stays
  unevaluated).
- Exact numeric arguments now fold during simplification (`sin(0) -> 0`,
  `exp(0) -> 1`, `ln(1) -> 0` and the family around zero and one), so
  results like `2*(sin(pi) - sin(0))` finish as `0`.
- Negation is normalized into the leading numeric coefficient of a product
  (`-(c * r) -> (-c) * r`), removing double-negation artifacts such as
  `-(-2 * exp(-1))` (now `2 * exp(-1)`).
- Graph sampling runs on a persistent background worker that reuses one
  interpreter and replays each REPL submission only once (previously every
  re-sample rebuilt the session from scratch).
- Sampling requests supersede each other (newest window wins) and out-of-date
  runs are cancelled cooperatively mid-batch (`sample_source_cancellable` in
  `src/plot.rs`) when the function list or session changes.
- Two-tier sampling: a fast pass keeps the graph fresh during pan/zoom
  gestures; a full-quality pass sharpens the curves once the view settles.
- The tap probe snaps once per tap in data coordinates (previously it
  re-scanned every sample on every frame).
- Zooming is clamped to sane window spans so ticks, sampling, and mapping stay
  well-defined during long wheel gestures.
- Duplicate Cmd+S handler removed; completion scans are skipped while the
  editor is unfocused; analysis debounce lowered 900 ms → 400 ms for a
  snappier preview and diagnostics.
- Graph-input definitions (`g(t) = 2t + 1`) now execute in the live REPL
  session and are replayed by the sampling worker (previously they only
  existed inside throwaway sampling interpreters).
