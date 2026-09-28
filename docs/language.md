# SCAS language reference

SCAS is a lexically scoped expression language with exact integer and rational
arithmetic, arbitrary-precision decimals, arrays/matrices, functions, control
flow, and symbolic values. The normative production grammar is
[`src/grammar.org`](../src/grammar.org).

Statements use semicolons except brace-delimited blocks and control-flow forms.
A `for` header is `for initializer; condition; update { ... }`. Conditions use
truthiness: only `false` and `nil` are false. `and` and `or` short-circuit.

Operators, tightest first, are postfix call/index, `^` (right associative),
unary `! - ~`, `* / %` and implicit multiplication, `+ -`, shifts, `&`, `xor`,
`|`, comparisons, equality, `and`, and `or`. Adjacency such as `3x^2` means
`3 * (x^2)`; a named call such as `f(x)` remains a call.

Arrays use `[a, b, c]`. Matrices use comma-separated columns and semicolon row
separators, for example `[1, 2; 3, 4]`. `A[i, j]` is equivalent to `A[i][j]`.
Rows must be rectangular. Names may contain underscores, so `x_1` is one name.

The real numeric promotion order is integer, rational, decimal; a complex
operand promotes arithmetic to complex. Integer division is an exact reduced
rational. A decimal operand requests decimal arithmetic.
Symbolic names are normalized automatically; explicit transforms include
`simplify`, `collect`, `expand`, `factor`, `apart`, and `horner`. `subs`,
`solve`, `derivative`, and `integral` provide substitution, rational polynomial
roots, differentiation, and elementary antiderivatives. Calculus operations
return an unevaluated call when no safe rule is known.

`pi`, `e`, and `tau` are exact symbolic constants. Exact function inputs remain
exact or symbolic; `evalf(expression)` requests an arbitrary-precision decimal
or complex approximation. A decimal input also requests numerical evaluation.
Elementary functions use principal complex branches when a result leaves the
real domain. The real special functions `gamma`, `lgamma`, `erf`, and `erfc`
are arbitrary precision; their complex extensions are explicitly unsupported.
See [`math-functions.md`](math-functions.md) for the support matrix and branch
conventions.

`assume_real(x)`, `assume_integer(x)`, `assume_nonzero(x)`,
`assume_nonnegative(x)`, `assume_positive(x)`, and `assume_negative(x)` attach
domain facts to a symbol in the current interpreter session. Conditional
rewrites require those facts: unconstrained `x/x` and `sqrt(x^2)` remain
unchanged, a real `x` permits `sqrt(x^2) -> abs(x)`, and a nonnegative `x`
permits `sqrt(x^2) -> x`. Contradictory sign assumptions are errors.
Assumptions are never inferred from unrelated expressions.
