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

The numeric promotion order is integer, rational, decimal. Integer division is
an exact reduced rational. A decimal operand requests decimal arithmetic.
Symbolic names are normalized automatically; explicit transforms include
`simplify`, `collect`, `expand`, `factor`, `apart`, and `horner`. `subs`,
`solve`, `derivative`, and `integral` provide substitution, rational polynomial
roots, differentiation, and elementary antiderivatives. Calculus operations
return an unevaluated call when no safe rule is known.

The numeric domain is real. Transcendental approximations use `f64`; exact
integer/rational algebra remains arbitrary precision. `sqrt(x^2)` normalizes to
`abs(x)`. Cancellation and inverse logarithm/exponential rewrites use the
documented generic-domain convention: symbolic denominators are assumed nonzero
and logarithm arguments are assumed in the positive real domain. Complex
numbers and branch-sensitive complex rewrites are unsupported.

`assume_nonnegative(x)` and `assume_nonzero(x)` attach domain facts to a
symbol in the current interpreter session. The simplifier uses these facts for
stronger safe forms such as `sqrt(x^2) -> x` and `abs(x) -> x`; without the
fact it retains `abs(x)`. Assumptions are explicit and are never inferred from
unrelated expressions.
