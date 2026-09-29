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
| Special | `erf`, `erfc`, `gamma`, `lgamma` | real |
| Utilities | `abs`, `sign`, `floor`, `ceil`, `round`, `trunc`, `min`, `max`, `hypot`, `degrees`, `radians` | real, except complex `abs` and angle conversion |
| Integer utilities | `gcd`, `lcm` | integers |
| Polynomial tools | `degree`, `coefficients`, `resultant`, `discriminant`, `polynomial_gcd`, `polynomial_divrem` | exact univariate polynomials over the rationals |
| Linear algebra | `transpose`, `trace`, `det`, `rank`, `rref`, `inverse`, `matmul`, `dot`, `cross`, `norm`, `solve_linear` | exact or symbolic vectors and rectangular matrices |

`lgamma(x)` means `ln(abs(gamma(x)))` for real non-pole inputs. The real
special functions use arbitrary-precision series, continued fractions,
reflection, recurrence, and asymptotic expansions as appropriate. Complex
`gamma`, `lgamma`, `erf`, and `erfc` are not implemented; SCAS returns a clear
runtime error rather than degrading their precision.

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
transition. The unsuffixed forms return only the result.

## Assumptions and safe simplification

An unbound symbol has no implicit real, positive, or nonzero property. Use the
assumption functions when a conditional identity is intended:

- `assume_real(x)`
- `assume_integer(x)` (also implies real)
- `assume_nonzero(x)`
- `assume_nonnegative(x)` (also implies real)
- `assume_positive(x)` (also implies real, nonnegative, and nonzero)
- `assume_negative(x)` (also implies real and nonzero)

For example, SCAS retains `x/x`, `sqrt(x^2)`, `exp(ln(x))`, and `ln(exp(x))`
for an unconstrained symbol. After `assume_real(x)`, it may use
`sqrt(x^2) = abs(x)` and `ln(exp(x)) = x`. After `assume_positive(x)`, all four
expressions simplify to their usual positive-real forms. Contradictory sign
assumptions are rejected.

Assumptions are session-local and currently attach to variables, not arbitrary
compound expressions. SCAS deliberately prefers an unsimplified correct
expression over a branch-unsafe rewrite.

## Calculus coverage

Differentiation includes algebraic powers; roots; exponential and logarithmic
functions; trigonometric, inverse-trigonometric, hyperbolic, and
inverse-hyperbolic functions; and `erf`, `erfc`, and assumption-qualified
`abs`.

Integration recognizes polynomial powers, affine reciprocals, affine roots,
exponential, sine, cosine, tangent, hyperbolic sine/cosine, `sec(x)^2`,
`csc(x)^2`, `sec(x)tan(x)`, and `csc(x)cot(x)`, including affine substitution.
It also covers logarithms, secant/cosecant, inverse trigonometric and inverse
hyperbolic functions, and `erf`/`erfc`. It returns an explicit unevaluated
`integral(...)` when no safe rule is known.

Multivariable and bounded-calculus tools are available as
`gradient(f, [x, y])`, `jacobian([f, g], [x, y])`,
`hessian(f, [x, y])`, and `definite_integral(f, x, a, b)`. Dynamic integral
rendering includes the constant of integration and a derivative check.

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
