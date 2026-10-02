# Algebraic identities for SCAS

This is a roadmap for the symbolic simplifier. Checked identities are currently
implemented. An unchecked identity is a candidate, not a promise about current
behavior.

The simplifier must distinguish unconditional identities from identities that
need domain assumptions. For example, `x / x = 1` requires `x != 0`, and
`sqrt(x^2) = x` is false over the reals when `x < 0` (the result is `abs(x)`).
The assumptions system tracks facts such as `real(x)`, `x > 0`, `x != 0`, and
`integer(x)` instead of applying conditional rewrites blindly.

## Canonical arithmetic

- [x] `x + 0 -> x`, `0 + x -> x`
- [x] `x - 0 -> x`, `0 - x -> -x`
- [x] `x * 1 -> x`, `1 * x -> x`
- [x] `x * 0 -> 0`, `0 * x -> 0`
- [x] `x / 1 -> x`
- [x] `x ^ 0 -> 1`, `x ^ 1 -> x`
- [x] `-(-x) -> x`
- [x] Fold operations whose operands are both numeric constants
- [x] Put sums and products into a deterministic operand order
- [x] Flatten associative trees: `(a + b) + c -> a + b + c`
- [x] Flatten products: `(a * b) * c -> a * b * c`
- [x] Keep one numeric coefficient and one numeric constant per term/sum
- [x] Normalize signs so negation is represented in one consistent place

## Like terms and powers

- [x] `x + x -> 2*x`
- [x] `a*x + b*x -> (a + b)*x` for integer coefficients
- [x] `a*x - b*x -> (a - b)*x` for integer coefficients
- [x] `x - x -> 0`
- [x] `x * x -> x^2`
- [x] `x^a * x^b -> x^(a + b)` for integer exponents
- [x] `(x^a)^b -> x^(a*b)` for integer exponents
- [x] Collect rational and decimal coefficients as well as integers
- [x] `x^a / x^b -> x^(a-b)` only when `x != 0` is known
- [x] `x^n * y^n -> (x*y)^n` for integer `n`
- [x] Expand `(x + y)^n` for non-negative integer `n` via `expand`
- [x] Factor common terms: `a*x + a*y -> a*(x + y)` via `factor`

## Cancellation and fractions

- [x] `x / x -> 1` only when `x != 0` is known
- [x] `x % x -> 0` only when `x != 0` is known
- [x] Exact integer division produces a reduced rational
- [x] Cancel common factors in rational symbolic expressions only when the
      cancelled denominator is known nonzero
- [x] Normalize a symbolic fraction to one numerator and denominator
- [x] Keep denominator signs positive
- [x] `a/b + c/d -> (a*d + b*c)/(b*d)` followed by GCD cancellation
- [x] Detect division by zero before cancellation
- [x] Partial fractions via `apart` for fractions with distinct rational
      linear factors (improper fractions are divided first; `together`
      recombines sums into one fraction as the inverse)

## Polynomial identities

- [x] `a*(b + c) -> a*b + a*c` via `expand`
- [x] `(a + b)*(c + d)` distributive expansion
- [x] `(a + b)^2 -> a^2 + 2*a*b + b^2`
- [x] `(a - b)^2 -> a^2 - 2*a*b + b^2`
- [x] `(a + b)*(a - b) -> a^2 - b^2`
- [x] `a^2 - b^2 -> (a-b)*(a+b)` via `factor`
- [x] `a^3 - b^3 -> (a-b)*(a^2 + a*b + b^2)`
- [x] `a^3 + b^3 -> (a+b)*(a^2 - a*b + b^2)`
- [x] Canonical sparse multivariate polynomial representation
- [x] Polynomial content/primitive-part normalization
- [x] Univariate polynomial GCD and square-free factorization over rationals
- [x] Horner form via `horner`

## Roots, absolute values, and powers

- [x] `sqrt(0) -> 0`, `sqrt(1) -> 1`
- [x] `sqrt(x^2) -> abs(x)` only for known real `x`
- [x] `sqrt(x^2) -> x` after `assume_nonnegative(x)`; otherwise retain the
      expression unless realness is known
- [x] Do not distribute `sqrt(a*b)` without valid domain assumptions
- [x] `abs(-x) -> abs(x)`
- [x] `abs(x) -> x` after `assume_nonnegative(x)`
- [x] `abs(x)^2 -> x^2` only when `x` is known real
- [x] Normalize exact numeric roots when numerator and denominator are perfect powers
- [x] Avoid unsafe rewrites of fractional powers across negative/complex values

## Exponentials and logarithms

- [x] `exp(0) -> 1`, `log(1) -> 0`
- [x] `log(exp(x)) -> x` only when `x` is known real
- [x] `exp(log(x)) -> x` only when `x` is known nonzero
- [x] `exp(a + b) -> exp(a)*exp(b)` when `expand` is requested
- [x] Do not expand `log(a*b)` without positive-real assumptions
- [x] Do not expand `log(a^n)` without branch/domain assumptions
- [x] `exp(a)*exp(b) -> exp(a+b)`

## Trigonometric identities

- [x] `sin(0) -> 0`, `cos(0) -> 1`, `tan(0) -> 0`
- [x] `sin(-x) -> -sin(x)`, `cos(-x) -> cos(x)`
- [x] `sin(x)^2 + cos(x)^2 -> 1`
- [x] `1 + tan(x)^2 -> sec(x)^2`
- [x] Exact common values at rational multiples of `pi` without numerical evaluation
- [x] Exact numeric arguments fold (`sin(0) -> 0`, `exp(0) -> 1`,
      `ln(1) -> 0`, and the closed-form family around zero and one)
- [x] Perfect rational powers fold (`sqrt(4) -> 2`, `cbrt(-8) -> -2`)
- [x] Angle-sum and double-angle expansion through `expand`
- [x] Keep exact trigonometric form symbolic; numerical complex arguments use
      principal complex evaluation
- [x] Respect exact poles and principal complex branches

## Calculus-oriented rewrites

- [x] Constant folding before differentiation or integration
- [x] Linearity of differentiation
- [x] Product, quotient, power, and elementary chain rules
- [x] Simplify derivatives immediately after constructing them
- [x] Recognize polynomial, reciprocal, exponential, sine, and cosine antiderivatives
- [x] Pull constant factors through integrals and recognize affine substitution
  for powers, reciprocals, roots, exponentials, sine, cosine, tangent,
  hyperbolic sine/cosine, and standard secant/cosecant products
- [x] Differentiate general powers plus inverse-trigonometric and hyperbolic functions
- [x] Differentiate `erf`, `erfc`, logarithms with fixed bases, roots, and
      assumption-qualified absolute values- [x] Bounded integration by parts for polynomial-times-exponential and
      polynomial-times-trigonometric forms: the tabular rule integrates
      `p(x) * g(linear)` for `g` in `exp`, `sin`, `cos`, `sinh`, `cosh`
      (the families closed under repeated integration), expanding at most 8
      polynomial degrees before giving up
- [x] Integrate rational functions through partial-fraction decomposition:
      proper fractions with distinct rational linear factors split through
      `apart` and integrate term by term (`c / (a*x + b) ->
      c * ln(a*x + b) / a` for constant numerators)
- [x] Trigonometric substitution for `c / (a*x^2 + b)` with positive
      rational `a`, `b` (the tangent substitution gives `atan` forms), and
      double-angle halves for `sin(a*x + b)^2` / `cos(a*x + b)^2` integrals
- [ ] General trigonometric substitution (Weierstrass-style) and product
      powers inside integrals (`∫ sin(x)^2*cos(x) dx` and friends)
- [x] Definite integrals: fundamental theorem on the current antiderivative
      set, linearity over sums of the integrand, and odd/even symmetry over
      symmetric intervals (`∫_{-a}^{a}` of an odd integrand vanishes; an even
      one folds to `2 * ∫_{0}^{a}`) — results with no closed primitive stay
      unevaluated instead of receiving corrupted substitutions
- [x] Keep unevaluated derivative/integral nodes when no rule applies
- [x] Definite constants/assumptions are not invented; indefinite results omit `+ C` by convention

## Boolean and comparison identities

- [x] `not not p -> p` for runtime boolean values
- [x] `p and true -> p`, `p and false -> false` through short-circuit evaluation
- [x] `p or false -> p`, `p or true -> true` through short-circuit evaluation
- [x] `p and p -> p`, `p or p -> p` for runtime boolean values
- [x] Constant-fold numeric comparisons
- [x] Do not derive comparisons from unavailable assumptions

## Implementation guidance

1. Canonicalize associative/commutative operations before adding many local
   rewrite rules. This prevents rule-order-dependent output.
2. Separate `simplify`, `expand`, and `factor`; automatic simplification should
   not repeatedly expand and refactor the same expression.
3. Attach domain requirements to conditional rewrites and return accumulated
   conditions when an identity changes the expression's domain.
4. Use an expression arena with hash-consing so structural equality and common
   subexpression reuse are cheap.
5. Add a termination measure or rewrite budget. Every automatic rule should
   move an expression toward a documented normal form.
6. Test identities both symbolically and by randomized numeric substitution,
   including zero, negative, rational, high-precision decimal, and complex
   boundary cases once those domains exist.
