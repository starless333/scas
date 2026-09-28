# Language and documenter decisions

These decisions close the open items in `DOCUMENTER.md`.

- Render delimiters are `\{ ... \}` and `\[ ... \]`; ordinary braces retain
  their existing execution and scope meaning.
- Render blocks share their surrounding scope so displayed definitions remain
  usable afterward.
- `///` inside a render block emits `\intertext{...}`. Four or more leading
  slashes are ordinary comments.
- Prose interpolation uses `#{ expression }`.
- Raw LaTeX uses a string in render position.
- Compound assignments render their expanded meaning, such as `a &= a + e`.
- Control flow in render blocks executes silently and produces an informational
  diagnostic.
- Vectors render as column `pmatrix` values by default.
- Matrix source syntax requires comma-separated columns and semicolon-separated
  rows: `[1, 2; 3, 4]`. Whitespace alone never separates elements.
- Subscripted names such as `x_1` are ordinary identifiers and render as
  `x_{1}`.

The lexer/parser implementation includes the render delimiters, show marker,
doc-comment token, and prerequisite expression grammar (matrix literals,
multi-index access, `else if`, exponent-aware implicit multiplication, and
subscript identifiers). Render constructs execute with their specified scope;
the LaTeX document assembler remains a separate, experimental output backend
described by `DOCUMENTER.md`.
