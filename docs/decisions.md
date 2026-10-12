# Language and documenter decisions

These decisions close the open items in `DOCUMENTER.md`.

- Render delimiters are `\{ ... \}` and `\[ ... \]`; ordinary braces retain
  their existing execution and scope meaning.
- Render blocks share their surrounding scope. Declarations and ordinary
  statements execute silently; only expressions marked with `=>` are typeset.
- `///` inside a render block closes the current alignment, renders as normal
  prose, and starts a fresh alignment for later `=>` rows. Four or more leading
  slashes are ordinary comments.
- Prose interpolation uses `#{ expression }`.
- Raw LaTeX uses a string in render position.
- Compound assignments inside render blocks execute silently.
- Control flow in render blocks executes silently and produces an informational
  diagnostic.
- Vectors render as column `pmatrix` values by default.
- Matrix source syntax requires comma-separated columns and semicolon-separated
  rows: `[1, 2; 3, 4]`. Whitespace alone never separates elements.
- Subscripted names such as `x_1` are ordinary identifiers and render as
  `x_{1}`.
- The desktop GUI uses `egui` through `eframe`. It is a Rust-native,
  cross-platform stack with a small application-state model and a path to a
  future WASM build. The first editor uses egui's built-in text buffer; a rope
  buffer can replace it when profiling shows that large files require one.
  Execution runs on a worker thread so evaluation never blocks painting.
- The live preview debounces edits and renders the generated source with the
  system `latex`, `dvisvgm`, and `rsvg-convert` commands, producing genuine
  Computer Modern output from SVG at the selected zoom. A small native
  structured-math renderer remains the fallback when those tools are
  unavailable. The export path remains canonical LaTeX.
- Greek editor shortcuts are stored as Unicode source, not merely painted as
  alternate glyphs. `π` and `τ` are executable aliases for the corresponding
  constants, and `Γ` is an alias for the Gamma function. Other Greek letters
  remain normal symbolic identifiers.
- Editor color schemes are UTF-8 TOML files with `[ui]` and `[syntax]` color
  tables. Colors use CSS-style `#RRGGBB` or `#RRGGBBAA` values. This keeps
  themes human-editable and makes every syntax category available without
  recompiling. Dark Modern is the default, with a bundled library of familiar
  dark and light developer palettes; external theme files remain supported.

The lexer/parser implementation includes the render delimiters, show marker,
doc-comment token, and prerequisite expression grammar (matrix literals,
multi-index access, `else if`, exponent-aware implicit multiplication, and
subscript identifiers). Render constructs execute with their specified scope;
the LaTeX document assembler remains a separate, experimental output backend
described by `DOCUMENTER.md`.
