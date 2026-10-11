# Self-Documenter Specification

Design document for the literate-programming layer of the language: source files that both **compute** and **typeset themselves** into a LaTeX document.

Status: grammar accepted; LaTeX assembly remains experimental. Design choices
are recorded in `docs/decisions.md`.

---

## 1. Goals and non-goals

### Goals
- A single source file is both the program and the document. What is typeset is what was executed.
- The **user decides** what gets rendered, using explicit markers. The renderer never guesses.
- Output is deterministic, total (every construct has some rendering), and always valid LaTeX.
- Output order equals source order, which makes source-to-preview navigation straightforward.
- Prose cannot silently disagree with computation (via interpolation).

### Non-goals
- Automatically detecting which expressions "should" be rendered.
- Generating textbook-style narrative for arbitrary problems (see optional extensions, section 10).
- Replacing LaTeX. Users can drop into raw LaTeX at any point.

---

## 2. Source model

| Construct | Meaning |
|---|---|
| `// ...` | Ordinary comment. Ignored by the interpreter and the documenter. |
| `/// ...` | **Document prose.** Raw LaTeX, emitted into the output in source order. |
| `{ ... }` | Silent block. Runs; nothing is typeset. Introduces a scope. |
| `\{ ... \}` | **Render block.** Runs its statements and typesets them as an `align*`, one row per statement. |
| `\[ ... \]` | **Render line.** Runs one expression and typesets it as a single display equation. |
| `=>` | **Show-result marker.** Typeset the expression, then `= <evaluated value>`. |
| `#{ expr }` | **Interpolation** inside `///` prose. Replaced by the typeset value of `expr`. |
| `plot(expr, lower, upper)` | Samples an expression of `x` and emits a vector graph into the LaTeX document. |
| `plot(expr, variable, lower, upper)` | Samples an expression of the named independent variable and emits a vector graph. |

### Example

```
/// # Problem 1
/// Find the derivative of $f(x) = x^2 + 2x$.
\{
  f(x) = x^2 + 2*x;
  derivative(f(x), x) =>;
\}
/// At $x = 3$ the slope is $#{ subs(derivative(f(x), x), x, 3) }$.
```

A plot can be used as a bare top-level statement or in a render line/block:

```
/// The parabola on $[-2, 2]$.
plot(x^2, -2, 2);
\\[ plot(sin(t), t, -pi, pi) \\]
\\{ plot(cos(x), -pi, pi); \\}
```

Plots are drawn with LaTeX vector primitives and become part of the normal
rendered document. Errors are reported as diagnostics with a visible
placeholder, allowing the remaining document to render.

Expected output:

```latex
\section*{Problem 1}
Find the derivative of $f(x) = x^2 + 2x$.
\begin{align*}
f(x) &= x^{2} + 2x \\
\frac{d}{dx} f(x) &= 2x + 2
\end{align*}
At $x = 3$ the slope is $8$.
```

---

## 3. Semantics

### 3.1 Doc comments
- Consecutive `///` lines form one paragraph. A bare `///` line is a paragraph break.
- One leading space after `///` is stripped.
- Contents are passed through as raw LaTeX, except for `#{ }` interpolation.
- `////` (four or more slashes) is a plain comment.
- A `///` line inside a render block is emitted as `\intertext{...}` so the `align*` continues.

### 3.2 Render blocks `\{ ... \}`
- Statements execute in order, exactly as in a normal block.
- Each semicolon-terminated statement produces a new aligned row. Commands
  chained with a comma share one row and are separated by `\quad`.
- **Scope: a render block does not introduce a new scope**, so definitions remain visible afterward.
- Rendering per statement kind:

| Statement | Typeset as |
|---|---|
| `let a = e;` | `a &= e` |
| `f(x) = e;` (math function decl) | `f(x) &= e` |
| `a = e;`, `a += e;` | `a &= e` (compound ops expand, e.g. `a &= a + e`) |
| `e;` | `e` (typeset as written, not evaluated for display) |
| `e =>;` | `e &= value` |
| `let a = e, b = f;` | `a = e \quad b = f` |
| `print(e);` | not typeset (executes only) |
| `return e;` | not typeset by default |
| `if` / `while` / `for` | executes; not typeset. Emit an informational diagnostic. |
| `fn` declarations | executes; not typeset, unless the user asks for it explicitly (see section 10) |

### 3.3 Render lines `\[ ... \]`
- Contains exactly one expression, optionally followed by `=>`.
- Emits `\[ ... \]` (or `equation*`).
- Without `=>`: typeset the expression as written. With `=>`, every builtin
  emits structured aligned work: evaluated arguments, its mathematical rule,
  and the exact result. Builtins with deeper traces include every available
  intermediate row. Linear-algebra derivations include the exact row operation
  beside each `A \sim A'` transition; integration adds `+C` and a
  differentiation check, shows by-parts or affine-substitution setup when
  applicable, and `solve(f, x)` factors before expanding its roots. Documented
  derivatives show the applied rule, and limits show direct substitution.
- Single-letter variables use normal italic mathematical notation. Longer
  identifiers are upright.

### 3.4 Interpolation `#{ expr }`
- `expr` is parsed with the normal expression grammar and evaluated in the current scope at that point in source order.
- The value is typeset with the same rules as render output.
- Evaluation errors become a diagnostic and render as a visible placeholder (`\text{\textbf{?}}`), never a crash.
- `#{` was chosen because bare `{` is pervasive in LaTeX.

### 3.5 Raw LaTeX escape hatch
Some notation will not be expressible in the expression grammar (`\vec{v}`, `\mathbb{R}`).
- String literals in render position pass through verbatim: `\{ "\\vec{v} = " ; \}`.
- The escape hatch is required for the tool to remain trustworthy.

---

## 4. Grammar additions

```
statement     ::= ... | render_block | render_line

render_block  ::= "\{" render_item* "\}"
render_item   ::= declaration
                | assignment_stmt
                | expr_stmt
                | show_stmt
                | statement             (* control flow etc. execute silently *)

render_line   ::= "\[" expression ("=>")? "\]"
show_stmt     ::= expression "=>" ";"

doc_comment   ::= "///" (any character except newline)*
interpolation ::= "#{" expression "}"     (* only inside doc_comment text *)
```

New tokens: `\{`, `\}`, `\[`, `\]`, `=>`, `DocComment`.

### Lexer requirements
- The backslash forms are distinct tokens, so there is no conflict with array `[`/`]` or block `{`/`}`.
- `//` remains trivia; `///` becomes a `DocComment` token that carries its text and span.
- Doc comment text is not tokenized, except to locate `#{ ... }` (track brace depth so `#{ f(x)[0] }` and nested braces work).
- Every token, including doc comments, carries a byte-range span.
- Maximal munch: `=>` must not be confused with `=` followed by `>` or with `>=`.

---

## 5. Rendering rules (expression to LaTeX)

The renderer is a pure function of the AST plus a small config. It must be **total**: an exhaustive `match` over every node type, so adding a node without a LaTeX rule fails to compile.

### 5.1 Core rules
- Precedence-aware parenthesization (pass the parent precedence down).
- Division becomes `\frac{}{}`; exponent becomes `^{}`; `sqrt(x)` becomes `\sqrt{}`.
- Implicit multiplication renders as juxtaposition (`2x`), explicit `*` renders as `\cdot` only when needed for clarity (number times number).
- Unary minus, factorial-like operators, comparison, and logical operators have explicit rules.
- Function calls: a template table of known functions; unknown functions fall back to `\operatorname{name}\left(...\right)`.

### 5.2 Function templates (user-extensible)

```toml
[functions]
derivative = '\frac{d}{d{1}}\left({0}\right)'
integral   = '\int {0}\,d{1}'
sin        = '\sin\left({0}\right)'
det        = '\det\left({0}\right)'
transpose  = '{0}^{\top}'
inv        = '{0}^{-1}'
```

### 5.3 Values
- Integers and rationals: `\frac{a}{b}`, with sign moved to the front.
- Floats: configurable precision.
- Matrices: `pmatrix`; vectors: column `pmatrix` by default (configurable).
- Lists of solutions: set notation or comma-separated, chosen by config.
- Booleans, strings, `nil`: `\text{true}`, `\text{...}`, `\varnothing`.

### 5.4 Identifiers
- Greek names map to commands: `alpha` becomes `\alpha`, `lambda` becomes `\lambda`.
- Underscore subscripts: `x_1` becomes `x_{1}`.
- Overridable via config.

---

## 6. Document assembly

1. Lex and parse the file into a sequence of items: `DocComment`, `RenderBlock`, `RenderLine`, and silent statements.
2. Walk the items **in source order** while executing the program, so scope and state are correct at each point.
3. Emit LaTeX fragments; concatenate.
4. Wrap in a preamble template (`amsmath`, `amssymb`, `booktabs`, plus user-supplied packages).
5. Optionally compile (Tectonic) or hand the fragments to the live preview.

Every emitted fragment records the **source span** it came from, enabling click-to-navigate in both directions.

---

## 7. Diagnostics

| Condition | Severity |
|---|---|
| Unterminated `\{` or `\[` | Error |
| More than one expression in `\[ \]` | Error |
| Evaluation error inside `=>` or `#{ }` | Error, with a placeholder in the output |
| Control flow inside a render block | Info |
| Undefined identifier used in a render block | Error |
| Unknown function with no template (rendered via fallback) | Hint |
| Unbalanced braces in raw LaTeX prose | Warning (detected best-effort) |

All diagnostics carry spans and surface inline in the editor.

---

## 8. Editor and IDE integration

- Syntax highlighting: doc comments highlighted as LaTeX; render markers get a distinct color from `{ }`.
- Live preview re-renders from the parsed items; on a parse error, keep the last good render and show diagnostics.
- Source-to-preview and preview-to-source navigation via fragment spans.
- Hover inside a render block shows the evaluated value.
- LSP: diagnostics, completion (including function-template names), and hover reuse the same front-end.

---

## 9. Known grammar gaps to fix first

These affect the documenter directly because users now write math inside render markers.

1. **`3x^2`**: `implicit_product` only accepts an atom, so `^` does not bind to the variable. Make it take an `exponent` operand.
2. **`else if`**: not in the grammar; add it.
3. **Matrix literals**: `[1 2; 3 4]` needs its own rule (`;` row separators). Whitespace-separated elements make `[1 -1]` ambiguous; decide on Julia-style whitespace rules.
4. **Indexing**: `A[i, j]` needs a comma-separated index list in `postfix`.
5. **Subscripted identifiers**: decide whether `x_1` is a plain identifier (it currently should be) and how it maps to LaTeX.

---

## 10. Optional extensions (after the core is solid)

- `\steps[ expr ]`: expand a rewrite trace for the expression, rendered generically as `before &= after && \text{(rule)}`.
- `\code{ ... }` or similar: typeset a function or algorithm as source or pseudo-code.
- `check` / `assert` statements with a gutter indicator for answer verification.
- Model-assisted features (natural-language to code, trace narration), always behind a verifier and never the default path.
- Multi-file projects and imports.

---

## 11. Testing strategy

- **Golden tests**: a corpus of `.cas` files with expected `.tex` output (use `insta`).
- **Compile test**: every golden output is compiled with Tectonic in CI to confirm the TeX is valid.
- **Property tests**: random expressions render without panics; render output has balanced braces; simplify is idempotent.
- **Round-trip**: parse, pretty-print, parse yields an identical AST.
- **Fuzzing**: `cargo fuzz` on the lexer (especially `#{ }` and doc comments) and parser.

---

## 12. Open decisions summary

Reviewed 2026-10-08: all decisions below remain implemented. The still-open
golden, TeX compile, and round-trip tests are tracked in `CAS_IDE.md`.

- [x] Delimiter syntax: `\{ \}` / `\[ \]`
- [x] Render block scope: shared
- [x] `/// ` lines inside render blocks: `\intertext`
- [x] Interpolation delimiter: `#{ }`
- [x] Raw LaTeX escape syntax: string in render position
- [x] Compound assignment renders its expanded meaning
- [x] Control flow inside render blocks executes silently with an info diagnostic
- [x] Default vector orientation: column
- [x] `////` is an ordinary comment
