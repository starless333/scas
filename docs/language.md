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
Rows must be rectangular. Matrix multiplication uses `A * B`; multiplying a
matrix by a vector with `A * v` treats the vector as a column vector. The
equivalent explicit forms are `matmul(A, B)` and `matmul(A, v)`. Names may
contain underscores, so `x_1` is one name. Matrix transpose may be written as
either `A'` or `transpose(A)`; the apostrophe is a chainable postfix operator.
Square matrices may be raised to bounded integer powers with `A^n`; zero gives
the identity and negative exponents use the exact matrix inverse.

An indexed rule constructs a matrix without spelling out every entry. In a
one-parameter function, `(expression)_(i,j)` uses the parameter as both
dimensions. Indices in the rule are one-based, while indexing the resulting
array remains zero-based:

```scas
let A(n) = (i + j)_(i,j);
print(A(3));              // [[2,3,4], [3,4,5], [4,5,6]]
print(det(A(2)));         // -1
```

Use `(expression)_(i,j,size)` for an explicit square size or
`(expression)_(i,j,rows,columns)` for a rectangle. The index names shadow
outer bindings, including the predefined imaginary unit `i`. Entries may
contain other symbolic variables, and the resulting matrix works with the
ordinary exact linear algebra and entrywise `limit` operations.

The real numeric promotion order is integer, rational, decimal; a complex
operand promotes arithmetic to complex. Integer division is an exact reduced
rational. A decimal operand requests decimal arithmetic.
Symbolic names are normalized automatically; explicit transforms include
`simplify`, `collect`, `expand`, `factor`, `apart`, and `horner`. `subs`,
`solve`, `derivative`, and `integral` provide substitution, rational polynomial
roots, differentiation, and elementary antiderivatives. Calculus operations
return an unevaluated call when no safe rule is known.

`verify_chain([a, b, ...])` checks each adjacent pair with exact polynomial
arithmetic or structural equality and stops at the first unproved link.
`rewrite(expr, pattern, replacement)` applies a single top-down pass. Unbound
names in the pattern are wildcards; repeated names must match the same expression,
and every name in the replacement must be bound. It is a user-specified
transformation, so SCAS does not assume the rule preserves equality.
`solve_periodic(expr, x, n)` gives affine sine, cosine, and tangent solution
families in a map with `families`, `parameter`, and `domain` (`"Z"`) fields.

`pi`, `e`, and `tau` are exact symbolic constants. Exact function inputs remain
exact or symbolic; `evalf(expression)` requests an arbitrary-precision decimal
or complex approximation. A decimal input also requests numerical evaluation.
Elementary functions use principal complex branches when a result leaves the
real domain. The real special functions `gamma`, `lgamma`, `erf`, and `erfc`
are arbitrary precision. Complex `gamma` and `lgamma` use reflection and
Stirling methods; complex `erf` and `erfc` use series and asymptotic methods.
See [`math-functions.md`](math-functions.md) for the support matrix and branch
conventions.

The document renderer recognizes `plot` calls in ordinary source files and
embeds the resulting graph as vector drawing commands in the same LaTeX
document as other math. Use `plot(expression, lower, upper)` for the default
independent variable `x`, or `plot(expression, variable, lower, upper)` to
choose another variable. For example:

```scas
/// The curve $y = x^2$ on $[-2,2]$.
plot(x^2, x, -2, 2);
\[ plot(sin(t), t, -pi, pi) \]
\{
    plot(cos(x), -pi, pi) =>;
\}
```

A bare top-level plot, render-line plot, or `=>` plot expression in a render
block is emitted as a graph; marked plots in render blocks appear on their own
aligned row. Other statements in a render block execute silently.
The plot expression is numerically sampled while preserving the surrounding
interpreter state. Invalid bounds or expressions produce a diagnostic and a
visible placeholder instead of stopping the rest of the document.

`assume_real(x)`, `assume_integer(x)`, `assume_nonzero(x)`,
`assume_nonnegative(x)`, `assume_positive(x)`, and `assume_negative(x)` attach
domain facts to a symbol in the current interpreter session. The general form
`assume(constraint)` accepts comparisons over one variable — `assume(x == 3)`,
`assume(x > 0 and x < 10)`, `assume(x != 0)` — recording each as a fact.
Equality facts substitute into later expressions, so after `assume(x == 3)`
the expression `x + 1` evaluates to `4`, and later `assume` calls whose
comparison folds to a concrete boolean are checked for consistency rather
than re-recorded. Contradictions between facts are errors, and
`forget_assumptions()` clears every fact. Conditional rewrites require those
facts: unconstrained `x/x` and `sqrt(x^2)` remain unchanged, a real `x`
permits `sqrt(x^2) -> abs(x)`, and a nonnegative `x` permits `sqrt(x^2) -> x`.
Assumptions are never inferred from unrelated expressions.
