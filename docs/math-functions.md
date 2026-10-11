# Mathematical functions

SCAS separates exact symbolic evaluation from requested numerical
approximation. This avoids silently replacing exact expressions with
machine-precision values and gives every numerical result a visible precision
contract.

## Exactness and `evalf`

`pi`, `e`, `tau`, and the golden ratio `phi` (also `π`, `τ`, and `φ`) are
symbolic constants. Exact integer and rational inputs
remain exact whenever SCAS knows the result, and otherwise remain symbolic.

```scas
print(sin(pi / 6));    // 1/2
print(log(8, 2));      // 3
print(gamma(6));       // 120
print(phi^2);          // phi + 1
print(sqrt(2));        // sqrt(2)
print(evalf(sqrt(2))); // decimal at the active precision
print(sqrt(2.0));      // decimal input also requests approximation
```

The active precision is a significant-digit target. It defaults to 100 and is
set with `--precision N` in batch mode or `:precision N` in the REPL. Internal
guard digits are used where an algorithm needs them; the displayed result is
rounded back to the requested precision.

## Numeric domains and branches

Elementary numerical functions operate on arbitrary-precision decimal real or
complex values. No elementary or special-function path converts through
`f32` or `f64`.

When an elementary result is not real, SCAS returns the principal complex
value instead of reporting a real-domain error. The branch conventions are:

- `sqrt`, `ln`, `log10`, `log2`, inverse trigonometric functions, inverse
  hyperbolic functions, and non-integral powers use their principal complex
  branches (branch cut along the conventional negative real axis where
  applicable).
- `cbrt` preserves the real cube root for real inputs; complex inputs use the
  principal cube root.
- `log(value, base)` is `ln(value) / ln(base)` on the same principal branches.
- `atan2(y, x)`, `hypot(x, y)`, ordering, `min`, `max`, `sign`, and integer
  rounding operations require real inputs.
- Exact poles such as `tan(pi/2)` are errors. Numerical results that are
  infinite or unrepresentable are errors rather than sentinel values.

Complex values print as `i`, `-i`, `bi`, or `a +/- bi`. Arithmetic `+`, `-`,
`*`, `/`, integer powers, equality, and `abs` work directly on complex values.
Complex numbers are intentionally unordered.

## Function support

| Family | Functions | Numeric domain |
| --- | --- | --- |
| Constants | `pi`, `e`, `tau`, `phi` | exact symbolic; real approximation through `evalf` |
| Roots and powers | `sqrt`, `cbrt`, `^` | real and complex |
| Exponential/logarithmic | `exp`, `ln`, `log`, `log10`, `log2` | real and complex |
| Trigonometric | `sin`, `cos`, `tan`, `sec`, `csc`, `cot` | real and complex |
| Inverse trigonometric | `asin`, `acos`, `atan`, `atan2` | complex except real-only `atan2` |
| Hyperbolic | `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh` | real and complex |
| Special | `erf`, `erfc`, `gamma`, `lgamma` | real and complex |
| Utilities | `abs`, `sign`, `floor`, `ceil`, `round`, `trunc`, `min`, `max`, `hypot`, `degrees`, `radians` | real, except complex `abs` and angle conversion |
| Integer utilities | `gcd`, `lcm` | integers |
| Polynomial tools | `degree`, `coefficients`, `resultant`, `discriminant`, `polynomial_gcd`, `polynomial_divrem` | exact univariate polynomials over the rationals |
| Linear algebra | `transpose`, `trace`, `det`, `rank`, `rref`, `inverse`, `matmul`, `dot`, `cross`, `norm`, `solve_linear` | exact or symbolic vectors and rectangular matrices |

`lgamma(x)` means `ln(abs(gamma(x)))` for real non-pole inputs. The real
special functions use arbitrary-precision series, continued fractions,
reflection, recurrence, and asymptotic expansions as appropriate. Complex
`erf` and `erfc` use an arbitrary-precision series near the complex origin
and an asymptotic expansion for larger arguments. Complex `gamma` uses
reflection, recurrence, and a Stirling expansion. Complex `lgamma` is the
principal logarithm of `gamma`. A difficult argument can produce a convergence
error at the requested precision.

## Polynomial invariants and Euclidean operations

The polynomial tools take the variable explicitly. `coefficients` returns the
coefficient list in ascending degree, and `polynomial_divrem` returns
`[quotient, remainder]`:

```scas
print(resultant(x^2 + 1, x + 1, x));       // 2
print(discriminant(x^2 + 3x + 2, x));       // 1
print(coefficients(3x^2 + 2x - 1, x));     // [-1, 2, 3]
print(polynomial_gcd(x^3-x, x^2-1, x));    // x^2 - 1
print(polynomial_divrem(x^3-1, x-1, x));   // [x^2 + x + 1, 0]
```

These operations are exact over rational coefficients. Resultants use an exact
rational Sylvester determinant and reject matrices above a fixed dimension
rather than allowing an accidental unbounded GUI workload.

## Linear algebra

Matrices use the existing row-separated literal syntax and vectors use ordinary
arrays. Linear algebra therefore works with exact integers, rationals, and
symbolic entries without introducing a separate numeric matrix type.

```scas
let A = [1, 2; 3, 4], b = [5, 11];
print(det(A));                 // -2
print(inverse(A));             // [[-2, 1], [3/2, -1/2]]
print(solve_linear(A, b));     // [1, 2]
print(dot([1, 2], [3, 4]));    // 11
print(A * A);                  // [[7, 10], [15, 22]]
print(A * b);                  // [27, 59]
print(A');                     // [[1, 3], [2, 4]]
```

Matrix products are available through either operator syntax or the `matmul`
builtin. `A * B` and `matmul(A, B)` perform matrix × matrix multiplication;
`A * v` and `matmul(A, v)` treat the vector as a column vector. Inner
dimensions must align.
The postfix form `A'` is equivalent to `transpose(A)` and binds more tightly
than exponentiation and multiplication.

Square matrices accept bounded integer powers through `A^n`. SCAS uses binary
exponentiation, returns the identity for `A^0`, and computes negative powers
from the exact inverse. Negative powers of singular matrices are errors.

Every operation also has a documenting form with a `_steps` suffix:
`transpose_steps`, `trace_steps`, `det_steps`, `rank_steps`, `rref_steps`,
`inverse_steps`, `matmul_steps`, `dot_steps`, `cross_steps`, `norm_steps`, and
`solve_linear_steps`. The explicit names return structured work when called
directly; ordinary calls such as `rref(A) =>;` automatically select that
documented execution. Row reduction records and displays every row
swap, scale, and row replacement beside the corresponding `A \sim A'`
transition. Determinants also show the triangular reduction and pivot product.
More generally, every builtin rendered with `=>` shows evaluated arguments,
the mathematical rule applied, and the exact result; specialized traces add
all intermediate work available. The unsuffixed forms return only the result
during ordinary, non-document execution.

## Constraints and assumptions

An unbound symbol has no implicit real, positive, or nonzero property. SCAS
deliberately prefers an unsimplified correct expression over a branch-unsafe
rewrite, so every conditional identity is gated on an explicit constraint.

### Named helpers

Each helper records a fact about one variable and returns that variable, so
calls can be chained or placed inline:

| Call | Fact recorded | Also implies |
| --- | --- | --- |
| `assume_real(x)` | `x` is real | — |
| `assume_integer(x)` | `x` is an integer | real |
| `assume_nonzero(x)` | `x ≠ 0` | — |
| `assume_nonnegative(x)` | `x ≥ 0` | real |
| `assume_positive(x)` | `x > 0` | real, nonnegative, nonzero |
| `assume_negative(x)` | `x < 0` | real, nonzero |

### Constraint form: `assume(constraint)`

The general form accepts a single comparison over one variable, or an `and`
combination of comparisons. The comparison operators are `==`, `!=`, `<`,
`<=`, `>`, and `>=`:

```scas
assume(x == 3);        // equality: substitutes into later expressions
assume(y > 0 and y < 10);
assume(z != 0);
forget_assumptions();  // clears every recorded fact
```

Each comparison must put a plain variable on one side — or be linear in a
single variable, in which case it is normalized first: `assume(x + 1 > 0)`
records `x > -1`, and `assume(2 * x < 6)` records `x < 3`. Symbolic bounds
such as `y < x + 1` are allowed but never conflict-check numerically. A later
`assume` whose comparison folds to a concrete boolean through existing
equality facts is treated as a consistency check: `true` is accepted without
adding a fact, and `false` is rejected as a contradiction. Contradictions are
detected between every pair of recorded facts: equality versus a bound
(`assume(a > 2); assume(a == 3)` passes, `assume(c > 0); assume(c == 0)` is
rejected), two numeric bounds with an empty interval (`assume(b > 2);
assume(b < 1)`), and sign claims across the named tools and `assume`
(`assume(d == 0); assume_positive(d)` names `d` in the error). Range facts
drive the same domain-safe simplifications as the named helpers
(`assume(y > 0)` unlocks the positive-real rewrites).

Equality facts are definitional: after `assume(x == 3)`, later expressions
see `x` as `3`. `forget_assumptions()` clears every recorded fact at once.

### What each fact unlocks

For an unconstrained symbol SCAS retains `x/x`, `sqrt(x^2)`, `exp(ln(x))`,
and `ln(exp(x))` verbatim. The rewrites unlocked by each fact:

| Fact | Identities it permits |
| --- | --- |
| real | `ln(exp(x)) -> x`, `sqrt(x^2) -> abs(x)` |
| nonzero | `x/x -> 1`, `x % x -> 0`, `x^a/x^b -> x^(a-b)`, `exp(ln(x)) -> x` |
| nonnegative | `abs(x) -> x`, `sqrt(x^2) -> x` |
| positive | all of the above, and the positive-log/exponent forms (`log(a^n) -> n*log(a)` for `a > 0`) |
| integer | parity and divisibility reasoning where relevant |

Assumptions are session-local and currently attach to variables, not
arbitrary compound expressions. The constraint system is also what qualifies
`abs` differentiation (`derivative(abs(x), x)` needs a sign fact) and the
symmetry shortcuts in `definite_integral`.

## Calculus: derivative and integral coverage

### `derivative(expr, variable)`

Differentiation is closed over the whole elementary language:

- **Structure**: constants, variables (other names are constants with respect
to `variable`), sums/differences, constant multiples, products (product
rule), quotients (quotient rule), and general powers `f^g` via logarithmic
differentiation (`derivative(x^x, x)` works). Linearity is applied first and
the result is simplified immediately.
- **Composed functions** (chain rule through any affine or general inner
expression): `sqrt`, `cbrt`, general powers `x^n`, `exp`, `ln`, `log10`,
`log2`, `sin`, `cos`, `tan`, `sec`, `csc`, `cot`, `asin`, `acos`, `atan`,
`sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`, `erf`, and `erfc`.
- **Assumption-qualified**: `abs(x)` differentiates to `sign(x)` only when a
sign fact is known; otherwise the derivative is kept unsimplified.
- **Higher derivatives** come from repeated calls
(`derivative(derivative(f, x), x)`).

Anything outside these rules remains as an explicit unevaluated
`derivative(...)` rather than guessing.

### `integral(expr, variable)`

Integration applies linearity first (sums split, constant factors pull out),
then matches the integrand against the rule set below. Affine arguments
`a*x + b` are handled everywhere with the correct `1/a` substitution factor.

| Integrand | Result family |
| --- | --- |
| `c * (a*x + b)^n`, integer `n ≠ -1` | power rule |
| `c / (a*x + b)` (constant numerator) | `c * ln(a*x + b) / a` |
| `sqrt(a*x + b)`, `cbrt(a*x + b)` | root rule |
| `exp(a*x + b)` | exponential |
| `ln(a*x + b)`, `log10(...)`, `log2(...)` | by-parts logarithm forms |
| `sin`, `cos`, `tan`, `sec`, `csc` of `a*x + b` | trigonometric |
| `sinh`, `cosh` of `a*x + b` | hyperbolic |
| `asin`, `acos`, `atan` of `a*x + b` | inverse trigonometric |
| `asinh`, `acosh`, `atanh` of `a*x + b` | inverse hyperbolic |
| `erf(a*x + b)`, `erfc(a*x + b)` | error functions |
| `sec^2`, `csc^2` of `a*x + b` | tangent/cotangent |
| `sec*tan`, `csc*cot` (same affine argument) | secant/cosecant |
| `p(x) * g(a*x + b)`, `g` in `exp`, `sin`, `cos`, `sinh`, `cosh` | **by parts** (tabular rule, polynomial degree ≤ 8) |
| rational with distinct rational linear factors | **partial fractions** (`apart` output feeds the rules above; improper fractions are divided first) |
| `c / (a*x^2 + b)`, positive rational `a`, `b` | tangent substitution: `c * atan(x / sqrt(b/a)) / (a * sqrt(b/a))` |
| `sin(a*x + b)^2`, `cos(a*x + b)^2` | double-angle halves: `x/2 ∓ sin(2*(a*x + b))/(4*a)` |

Examples of the newer families:

```scas
print(integral(x*exp(x), x));                  // x*exp(x) - exp(x)
print(integral(x^2*cos(x), x));                // x^2*sin(x) + 2x*cos(x) - 2sin(x)
print(integral((3x + 1)/((x - 1)*(x + 2)), x)); // 4/3*ln(x - 1) + 5/3*ln(x + 2)
print(integral(1/(x^2 + 1), x));               // atan(x)
print(integral(sin(x)^2, x));                  // x/2 - sin(2x)/4
```

Improper rational functions, repeated or irreducible-quadratic denominator
factors, and products of several transcendentals (for example
`sin(x)*cos(x)`) are not yet decomposed; those return an explicit
unevaluated `integral(...)`. Nested or out-of-budget forms are never guessed.

### `definite_integral(expr, variable, lower, upper)`

- Evaluates `F(upper) - F(lower)` whenever `integral` finds a closed
antiderivative (the fundamental theorem on the current rule set).
- **Linearity**: sums split per term and each term gets the best strategy.
- **Symmetry**: when `lower == -upper`, an odd integrand evaluates to `0`
(even without a closed antiderivative) and an even integrand folds to
`2 * ∫_{0}^{upper}`.
- When no closed primitive exists, the call stays as an explicit
unevaluated `definite_integral(...)`; SCAS never substitutes limits into an
unevaluated integral.

### Multivariable tools

`gradient(f, [x, y])`, `jacobian([f, g], [x, y])`, and `hessian(f, [x, y])`
build on the same derivative engine. Dynamic integral rendering in documents
includes the constant of integration and a derivative check.

`multiple_integral(f, [x, y, ...])` performs repeated symbolic integration in
the listed order. `limit(f, x, a)` uses direct substitution followed by up to
eight L'Hôpital steps for removable `0/0` forms; an optional fourth argument
is `"left"` or `"right"`. `series(f, x, a, n)` returns the degree-`n` Taylor
polynomial. `sum(f, k, a, b)` and `product(f, k, a, b)` evaluate finite integer
ranges exactly. Symbolic sums of `1`, `k`, `k^2`, and `k^3` use Faulhaber
forms; unsupported symbolic bounds remain unevaluated.

`dsolve([a, b], x)` and `dsolve([a, b, c], x)` return a fundamental solution
basis for homogeneous constant-coefficient first- and second-order ODEs.
`dsolve(p, q, x, C)` solves the first-order linear equation
`y' + p(x)y = q(x)` with the supplied integration-constant symbol `C`.

## Standard library reference

Every builtin registered by SCAS 0.1.0, grouped by family. Signatures use
`x` for a value, `expr` for a symbolic expression, `v` for a vector, and `A`
for a matrix. The sections above give the domains; this table is the
inventory.

### Constants

| Name | Meaning | Notes |
| --- | --- | --- |
| `pi` (`π`) | exact π | `evalf(pi)` approximates |
| `e` | exact Euler number | |
| `tau` (`τ`) | exact 2π | |
| `phi` (`φ`) | exact golden ratio | `phi^2 -> phi + 1` is recognized |
| `i` | complex unit | `i^2 = -1` |
| `Γ` | uppercase gamma | alias of `gamma` |

### Numeric utilities

| Signature | Result |
| --- | --- |
| `abs(x)` | absolute value or complex magnitude |
| `sign(x)` | −1, 0, or 1 (real) |
| `floor(x)`, `ceil(x)`, `round(x)`, `trunc(x)` | integer rounding (real) |
| `min(a, b)`, `max(a, b)` | ordered selection (real) |
| `hypot(x, y)` | stable √(x²+y²) (real) |
| `gcd(a, b)`, `lcm(a, b)` | exact integer operations |
| `degrees(x)`, `radians(x)` | angle conversion |
| `evalf(expr)` | approximation at the active precision |

### Elementary functions

| Signature | Notes |
| --- | --- |
| `sqrt(x)`, `cbrt(x)` | principal complex branches; real cube root preserved |
| `exp(x)`, `ln(x)` | principal complex branches |
| `log(x)` | natural logarithm |
| `log(x, base)`, `log10(x)`, `log2(x)` | `ln(x)/ln(base)` style, same branches |
| `sin`, `cos`, `tan`, `sec`, `csc`, `cot` | exact at rational multiples of `pi` |
| `asin`, `acos`, `atan` | inverse trigonometric |
| `atan2(y, x)` | real only |
| `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh` | hyperbolic family |

### Special functions

| Signature | Notes |
| --- | --- |
| `erf(x)`, `erfc(x)` | real and complex error functions; series and asymptotic branches |
| `gamma(x)`, `lgamma(x)` | real and complex; real `lgamma(x) = ln(abs(gamma(x)))`; complex `lgamma` uses the principal logarithm |

### Symbolic algebra

| Signature | Result |
| --- | --- |
| `simplify(expr)` | canonical simplification |
| `expand(expr)` | distribution and angle-sum expansion |
| `factor(expr)` | common-content and univariate factoring |
| `apart(expr)` | partial fractions (repeated rational linear factors and one remaining irreducible quadratic; improper fractions are divided first) |
| `together(expr)` | sum of fractions → single fraction (inverse of `apart`, shared content cancelled) |
| `horner(expr)` | Horner form |
| `collect(expr, x)` | collect by powers of `x` |
| `subs(expr, x, value)` | substitution |
| `solve(expr, x)` | exact roots where possible, numeric refinement otherwise |

### Constraints (see “Constraints and assumptions” above)

`assume(constraint)`, `assume_real(x)`, `assume_integer(x)`,
`assume_nonzero(x)`, `assume_nonnegative(x)`, `assume_positive(x)`,
`assume_negative(x)`, `forget_assumptions()`.

### Calculus (see “Calculus” above)

`derivative(expr, x)`, `integral(expr, x)`, `definite_integral(expr, x, a, b)`,
`limit(expr, x, a[, side])`, `series(expr, x, a, n)`,
`sum(expr, k, a, b)`, `product(expr, k, a, b)`,
`gradient(expr, [vars])`, `jacobian([exprs], [vars])`, `hessian(expr, [vars])`,
`multiple_integral(expr, [vars])`, `dsolve(coefficients, x)`, and
`dsolve(p, q, x, C)`.

### Polynomials

`degree(p, x)`, `coefficients(p, x)`, `resultant(p, q, x)`,
`discriminant(p, x)`, `polynomial_gcd(p, q, x)`,
`polynomial_divrem(p, q, x)` — exact univariate operations over the
rationals (see “Polynomial invariants and Euclidean operations”).

### Linear algebra

`transpose(A)`, `trace(A)`, `det(A)`, `rank(A)`, `rref(A)`, `inverse(A)`,
`matmul(A, B)`, `dot(u, v)`, `cross(u, v)`, `norm(v)`,
`solve_linear(A, b)` — exact or symbolic vectors and matrices (see
“Linear algebra”).

Spectral and vector-space tools are `characteristic_polynomial(A, x)`,
`eigenvalues(A)`, `eigenvectors(A)`, `null_space(A)`, `column_space(A)`,
`gram_schmidt([vectors])`, `projection(v, onto)`, and
`change_basis(v, [basis_vectors])`. Exact eigenvalues/eigenvectors currently
cover 1×1 and 2×2 matrices; eigenvalues of larger triangular matrices are read
from the diagonal.

Decompositions are `lu(A)`, `qr(A)`, and `cholesky(A)`. They return `[L, U]`,
`[Q, R]`, and `L` respectively. `matrix_exp(A)` currently evaluates diagonal
matrices exactly, and `jordan_form(A)` covers matrices through 2×2.

### Documenting variants

Every linear-algebra operation has a `_steps` form: `transpose_steps`,
`trace_steps`, `det_steps`, `rank_steps`, `rref_steps`, `inverse_steps`,
`matmul_steps`, `dot_steps`, `cross_steps`, `norm_steps`,
`solve_linear_steps`. The unsuffixed forms return only the result.
The decomposition variants `lu_steps`, `qr_steps`, and `cholesky_steps` also
produce renderer-visible derivations.

## Resource and error model

Ordinary decimal precision and transcendental precision have separate limits.
The defaults are 10,000 and 2,000 significant digits respectively. The CLI
flag `--max-transcendental-precision N` changes the latter, while Rust
embedders can configure both through `Limits`. Exceeding either budget returns
a structured `limit error`.

The numerical layer returns typed errors for invalid precision, poles,
unsupported complex special functions, and non-finite results. Public number
conversion APIs return `Option` when a complex value cannot be represented in
a real-only type; they do not silently drop the imaginary component.

## Verification

The regression suite covers exact special angles, principal complex branches,
assumption-dependent identities, calculus rules, precision limits, and
50-digit reference vectors for `gamma`, `lgamma`, `erf`, and `erfc`. The
reference vectors are independently checked with Python `mpmath`; the test
suite itself remains self-contained and does not require Python.
