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
- [ ] Put sums and products into a deterministic operand order
- [ ] Flatten associative trees: `(a + b) + c -> a + b + c`
- [ ] Flatten products: `(a * b) * c -> a * b * c`
- [ ] Keep one numeric coefficient and one numeric constant per term/sum
- [ ] Normalize signs so negation is represented in one consistent place

## Like terms and powers

- [x] `x + x -> 2*x`
- [x] `a*x + b*x -> (a + b)*x` for integer coefficients
- [x] `a*x - b*x -> (a - b)*x` for integer coefficients
- [x] `x - x -> 0`
- [x] `x * x -> x^2`
- [x] `x^a * x^b -> x^(a + b)` for integer exponents
- [x] `(x^a)^b -> x^(a*b)` for integer exponents
- [ ] Collect rational and decimal coefficients as well as integers
- [ ] `x^a / x^b -> x^(a-b)` when the required nonzero assumptions hold
- [ ] `x^n * y^n -> (x*y)^n` when valid for the active domain
- [ ] Expand `(x + y)^n` for small non-negative integer `n` on request
- [ ] Factor common terms: `a*x + a*y -> a*(x + y)`

## Cancellation and fractions

- [x] `x / x -> 1` under the generic-nonzero convention
- [x] `x % x -> 0` under the generic-nonzero convention
- [x] Exact integer division produces a reduced rational
- [ ] Cancel common factors in rational symbolic expressions
- [ ] Normalize a symbolic fraction to one numerator and denominator
- [ ] Keep denominator signs positive
- [ ] `a/b + c/d -> (a*d + b*c)/(b*d)` followed by GCD cancellation
- [ ] Detect and preserve division-by-zero conditions during cancellation
- [ ] Perform partial-fraction decomposition for rational functions

## Polynomial identities

- [ ] `a*(b + c) -> a*b + a*c` when expansion is requested
- [ ] `(a + b)*(c + d)` distributive expansion
- [ ] `(a + b)^2 -> a^2 + 2*a*b + b^2`
- [ ] `(a - b)^2 -> a^2 - 2*a*b + b^2`
- [ ] `(a + b)*(a - b) -> a^2 - b^2`
- [ ] `a^2 - b^2 -> (a-b)*(a+b)` when factoring is requested
- [ ] `a^3 - b^3 -> (a-b)*(a^2 + a*b + b^2)`
- [ ] `a^3 + b^3 -> (a+b)*(a^2 - a*b + b^2)`
- [ ] Canonical sparse multivariate polynomial representation
- [ ] Polynomial content/primitive-part normalization
- [ ] Polynomial GCD and square-free factorization
- [ ] Horner form for efficient numerical evaluation

## Roots, absolute values, and powers

- [ ] `sqrt(0) -> 0`, `sqrt(1) -> 1`
- [ ] `sqrt(x^2) -> abs(x)` for real `x`
- [ ] `sqrt(x^2) -> x` when `x >= 0` is known
- [ ] `sqrt(a*b) -> sqrt(a)*sqrt(b)` only with valid domain assumptions
- [ ] `abs(-x) -> abs(x)`
- [ ] `abs(x) -> x` when `x >= 0` is known
- [ ] `abs(x)^2 -> x^2` for real `x`
- [ ] Normalize rational powers by extracting perfect powers
- [ ] Avoid unsafe rewrites of fractional powers across negative/complex values

## Exponentials and logarithms

- [ ] `exp(0) -> 1`, `log(1) -> 0`
- [ ] `log(exp(x)) -> x` with the appropriate complex branch policy
- [ ] `exp(log(x)) -> x` when `x` is in the logarithm's domain
- [ ] `exp(a + b) -> exp(a)*exp(b)` when expansion is requested
- [ ] `log(a*b) -> log(a) + log(b)` only under suitable assumptions
- [ ] `log(a^n) -> n*log(a)` only when branch/domain conditions permit it
- [ ] Combine products of exponentials with equal bases

## Trigonometric identities

- [ ] `sin(0) -> 0`, `cos(0) -> 1`, `tan(0) -> 0`
- [ ] `sin(-x) -> -sin(x)`, `cos(-x) -> cos(x)`
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
