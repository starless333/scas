# Algebraic identities for SCAS

This is a roadmap for the symbolic simplifier. Checked identities are currently
implemented. An unchecked identity is a candidate, not a promise about current
behavior.

The simplifier must distinguish unconditional identities from identities that
need domain assumptions. For example, `x / x = 1` requires `x != 0`, and
`sqrt(x^2) = x` is false over the reals when `x < 0` (the result is `abs(x)`).
A future assumptions system should track facts such as `real(x)`, `x > 0`, and
`integer(n)` instead of applying conditional rewrites blindly.

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
- [x] `x^a / x^b -> x^(a-b)` under the generic-nonzero convention
- [x] `x^n * y^n -> (x*y)^n` for integer `n`
- [x] Expand `(x + y)^n` for non-negative integer `n` via `expand`
- [x] Factor common terms: `a*x + a*y -> a*(x + y)` via `factor`

## Cancellation and fractions

- [x] `x / x -> 1` under the generic-nonzero convention
- [x] `x % x -> 0` under the generic-nonzero convention
- [x] Exact integer division produces a reduced rational
- [x] Cancel common factors in rational symbolic expressions
- [x] Normalize a symbolic fraction to one numerator and denominator
- [x] Keep denominator signs positive
- [x] `a/b + c/d -> (a*d + b*c)/(b*d)` followed by GCD cancellation
- [x] Detect division by zero before cancellation
- [x] Partial fractions via `apart` for proper fractions with distinct rational
      linear factors

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
- [x] `sqrt(x^2) -> abs(x)` for real `x`
- [ ] `sqrt(x^2) -> x` when `x >= 0` is known
- [ ] `sqrt(a*b) -> sqrt(a)*sqrt(b)` only with valid domain assumptions
- [x] `abs(-x) -> abs(x)`
- [ ] `abs(x) -> x` when `x >= 0` is known
- [ ] `abs(x)^2 -> x^2` for real `x`
- [ ] Normalize rational powers by extracting perfect powers
- [ ] Avoid unsafe rewrites of fractional powers across negative/complex values

## Exponentials and logarithms

- [x] `exp(0) -> 1`, `log(1) -> 0`
- [ ] `log(exp(x)) -> x` with the appropriate complex branch policy
- [ ] `exp(log(x)) -> x` when `x` is in the logarithm's domain
- [ ] `exp(a + b) -> exp(a)*exp(b)` when expansion is requested
- [ ] `log(a*b) -> log(a) + log(b)` only under suitable assumptions
- [ ] `log(a^n) -> n*log(a)` only when branch/domain conditions permit it
- [ ] Combine products of exponentials with equal bases

## Trigonometric identities

- [x] `sin(0) -> 0`, `cos(0) -> 1`, `tan(0) -> 0`
- [x] `sin(-x) -> -sin(x)`, `cos(-x) -> cos(x)`
- [ ] `sin(x)^2 + cos(x)^2 -> 1`
- [ ] `1 + tan(x)^2 -> sec(x)^2`
- [ ] Exact values at rational multiples of `pi`
- [ ] Angle-sum and double-angle expansion on request
- [ ] Convert between trigonometric and exponential forms on request
- [ ] Respect poles and complex-domain behavior

## Calculus-oriented rewrites

- [ ] Constant folding before differentiation or integration
- [ ] Linearity: `D(a*f + b*g) -> a*D(f) + b*D(g)` for constants `a`, `b`
- [ ] Product, quotient, and chain rules
- [ ] Simplify derivatives immediately after constructing them
- [ ] Recognize common antiderivative patterns
- [ ] Keep unevaluated derivative/integral nodes when no rule applies
- [ ] Track integration constants and assumptions explicitly

## Boolean and comparison identities

- [ ] `not not p -> p`
- [ ] `p and true -> p`, `p and false -> false`
- [ ] `p or false -> p`, `p or true -> true`
- [ ] `p and p -> p`, `p or p -> p`
- [ ] Constant-fold numeric comparisons
- [ ] Derive comparisons from assumptions without guessing unknown relations

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
