# Where SCAS draws the exact/approximate boundary

Computer algebra becomes unreliable when a display approximation silently turns into an algebraic fact. SCAS therefore keeps integers and rational numbers as exact values. A calculation such as `1/3 + 1/6` reduces to `1/2`; it does not first round either operand to a machine float. Integer magnitudes are unbounded, and rationals use arbitrary-size numerators and denominators.

Decimal literals occupy a different role. They use decimal coefficients and scales, so writing `0.1` does not introduce the binary floating-point value nearest to one tenth. SCAS can convert a finite decimal to its exact rational value when an operation needs that representation. The distinction is still visible in the runtime type: `Number` has separate integer, rational, decimal, and complex variants.

The difficult boundary is a transcendental function. `pi` cannot be stored as a finite rational, and `sin(pi/6)` should not lose its exact value merely because a numerical evaluator knows how to approximate sine. The symbolic engine recognizes selected special angles and keeps unevaluated expressions for values it cannot prove exactly. `evalf` is the explicit request to evaluate a symbolic expression numerically at the active precision. An approximate result is useful for a plot or an estimate, but it cannot certify an identity.

This matters most for simplification. For example, cancelling `x/x` to `1` changes the expression at `x = 0`. Likewise, a square-root identity that holds for nonnegative real values need not hold for an unconstrained complex symbol. SCAS records assumptions separately and uses them only where a rule can justify them. The documenter's `assert` fails when a claim cannot be established, so a mathematical document cannot quietly publish a false computed claim. `verify_chain` applies an even narrower proof rule: it checks adjacent expressions by exact polynomial arithmetic or identical symbolic structure. It rejects an unproved link instead of testing a few sample points.

That choice has a cost. Some true identities are outside today's proof procedures, and a user may have to rewrite a step or state an assumption. The benefit is a clear contract: a displayed decimal is a chosen approximation, while an accepted exact transformation has an exact reason behind it.

## Try it

```scas
assert(1/3 + 1/6 == 1/2);
verify_chain([x^2 + 2*x + 1, (x + 1)^2]);
print(evalf(pi));
```

The implementation paths are [`Number`](../src/number/mod.rs), the [symbolic evaluator](../src/interpreter/interpreter.rs), and the [polynomial representation](../src/interpreter/polynomial.rs).
