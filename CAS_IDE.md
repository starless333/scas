# CAS IDE Checklist

A prioritized roadmap for the Rust CAS, its **explicit-markup self-documenter** (`///` prose, `\{ \}` render blocks, `\[ \]` render lines), and the IDE built around it. Work top to bottom. Do not start a phase until the previous one is solid.

See `docs/DOCUMENTER.md` for the full design of the documenter.

> **Change from the previous version:** the documenter no longer guesses what to render. The user marks it explicitly, so the old `// Question:` convention, automatic question/answer detection, and "question and code disagree" linting are replaced by `///` prose, render markers, and `#{ }` interpolation. Step-by-step solutions and AI features are now opt-in extras, not the default path.

---

## Phase 0: Foundations

### Language core readiness
- [x] Lexer and parser attach **source spans** to every token and AST node
- [x] Parser has **error recovery**: a half-typed line yields a partial AST plus diagnostics
- [x] Interpreter is exposed as a **library crate** (no `println!` or `exit`; returns structured `Result` errors)
- [x] Evaluation runs **without global side effects**, so the editor can re-run on every change
- [x] Symbolic engine has stable **canonical forms** and a working `simplify`

### Grammar fixes (these directly affect what users write inside render blocks)
- [x] `3x^2` binds correctly (implicit product takes an `exponent` operand)
- [x] Add `else if`
- [x] Matrix literals require comma-separated columns (`[1, 2; 3, 4]`), so `[1 -1]` remains subtraction
- [x] Multi-index `A[i, j]`
- [x] Decide identifier rules for subscripts (`x_1`) and Greek names

### Project structure
- [ ] **Cargo workspace**: `core` (lexer/parser/AST), `eval`, `latex` (renderer), `doc` (documenter), `lsp`, `gui`, `cli`
- [x] **CI** (GitHub Actions): build, test, `clippy`, `fmt --check`
- [x] Baseline **test suite** (parser, evaluator)
- [x] License, `.gitignore`, README stub

### Decisions to record in `docs/decisions.md`
- [x] Render marker syntax (`\{ \}` / `\[ \]` versus alternatives)
- [x] Render block scope (shared versus new)
- [x] Interpolation delimiter (`#{ }`)
- [x] Raw LaTeX escape hatch syntax
- [ ] GUI framework, text buffer approach, math rendering approach
- [ ] Theme file format

---

## Phase 1: Documenter core (the heart of the project)

### Lexing and parsing
- [x] `DocComment` token for `///` (with spans); `//` stays trivia
- [x] Tokens for `\{`, `\}`, `\[`, `\]`, and `=>`
- [x] Merge consecutive `///` lines into paragraphs; blank `///` is a paragraph break
- [x] Locate `#{ expr }` interpolations inside prose (brace-depth aware)
- [x] Grammar: `render_block`, `render_line`, `show_stmt`
- [x] Parser recovery for unterminated render markers

### Semantics
- [x] Execute render blocks in source order with the decided scoping
- [ ] `expr;` typesets as written; `expr =>;` typesets `expr = value`
- [ ] Interpolation evaluates in current scope; errors render a visible placeholder
- [ ] Silent execution of control flow inside render blocks, with an info diagnostic

### Renderer (AST and values to LaTeX)
- [x] **Total** source-expression renderer (exhaustive `match`; new nodes fail to compile without a rule)
- [x] Precedence-aware parenthesization
- [x] `\frac`, `^{}`, `\sqrt`, implicit multiplication as juxtaposition
- [ ] **User-extensible function template table** (TOML), with `\operatorname` fallback
- [ ] Values: integers, rationals, floats, booleans, strings, `nil`
- [x] Source matrices and vectors as `pmatrix`
- [ ] Identifier mapping (Greek names, subscripts), overridable
- [x] Raw LaTeX string escape hatch in the source-expression renderer

### Assembly
- [ ] Emit fragments in source order, each tagged with its **source span**
- [ ] Preamble template (`amsmath`, `amssymb`, `booktabs`, user packages)
- [ ] CLI: `mycas build file.cas` produces a `.tex` file

### Tests (start here, keep growing)
- [ ] **Golden tests**: `.cas` to expected `.tex` (`insta`)
- [ ] Compile every golden output with Tectonic in CI
- [ ] Property tests: renderer never panics, output braces balance
- [ ] Round-trip: parse, print, parse

---

## Phase 2: Core IDE (MVP)

- [ ] Text editor: cursor, selection, undo/redo, clipboard, scrolling
- [ ] **Syntax highlighting** from your lexer, with doc comments highlighted as LaTeX and render markers visually distinct from `{ }`
- [ ] **Arbitrary color schemes** (ship Gruber Darker plus one light theme; load custom themes from file)
- [ ] **File tree** sidebar
- [ ] Tabs or multiple buffers
- [ ] **Run button** and shortcut; output panel
- [ ] **Live LaTeX preview** pane, driven by documenter output
- [ ] **Debounced, off-thread rendering**
- [ ] Preview keeps the **last good render** on parse errors
- [ ] Save/open, dirty indicator, autosave or unsaved-changes prompt
- [ ] Line numbers, current-line highlight

---

## Phase 3: Language intelligence

- [ ] **Inline diagnostics** (squiggles, hover messages) using the documenter's diagnostics table
- [ ] Diagnostics panel
- [ ] **Autocomplete** for built-ins, user functions, and function-template names
- [ ] **Signature help**
- [ ] **Hover to evaluate** any expression
- [ ] **Go to definition**, **find references**, **rename symbol**
- [ ] **Click-to-navigate**: preview to source and source to preview using fragment spans (cheap now, since output order equals source order)
- [ ] **LSP server** (`tower-lsp`) exposing the above
- [ ] GUI as one LSP client; test the server in VS Code or Neovim

---

## Phase 4: Engineering credibility

- [x] **Property tests** on the simplifier (idempotence)
- [x] **Fuzz** the lexer and parser (`cargo fuzz`), especially doc comments and `#{ }`
- [ ] **Incremental parsing / evaluation caching**
- [x] **Benchmarks** (dependency-free harness) with numbers you can quote
- [x] **Release pipeline**: prebuilt binaries for Linux, macOS, Windows
- [ ] **WASM build** and a **public web playground**

---

## Phase 5: CAS-specific extras (opt-in, all built on the explicit model)

Pick one or two and do them well.

- [ ] **`\steps[ expr ]`**: expand a rewrite trace generically (`before &= after && \text{(rule)}`)
- [ ] Rewrite-rule tracing in the engine (power, sum, product, chain, constant multiple)
- [ ] Row-reduction traces for matrices
- [ ] **Graphing pane** for `f(x)` and its derivative
- [ ] **Inline results**: ghost annotation of evaluated values at the end of lines
- [ ] **`check` / `assert`** with gutter pass/fail indicators (replaces the old `// Answer:` idea)
- [ ] Typeset functions or algorithms from `fn` declarations

---

## Phase 6: Developer tooling

- [ ] **Command palette** (Ctrl+Shift+P)
- [ ] Configurable **keybindings**
- [ ] **REPL panel** sharing the file's environment
- [ ] **Formatter** (`fmt`) and format-on-save
- [ ] **Linter** (unused variables, shadowing, etc.)
- [ ] **Snippets/templates** (e.g. a problem skeleton with `///` prose and a render block)
- [ ] **Debugger / step-through evaluator**

---

## Phase 7: Export and workflow

- [ ] **PDF export** with bundled Tectonic
- [ ] CLI **watch mode**
- [ ] **Multi-file projects** with imports
- [ ] "Build whole problem set" producing a single document

---

## Phase 8: Optional AI layer (only after Phases 1 to 5 are solid)

The CAS remains the source of truth; models only translate and phrase.

- [ ] Backend interface that is swappable (hosted API or local model)
- [ ] **Natural language to source file** (generates `///`, render blocks, and code), followed by a run-and-check loop
- [ ] **Trace narration** as prose, schema-constrained
- [ ] **Verifier**: parse and check model-produced math against the CAS; compile the TeX; fall back to the deterministic output on failure
- [ ] Metrics: compile rate, equivalence rate, fallback rate
- [ ] Spending cap, rate limits, or bring-your-own-key for any public demo
- [ ] Fine-tuning a small model on synthetic CAS data (optional experiment)

---

## Phase 9: Presentation (before applying anywhere)

- [ ] README with a **5-second demo GIF** at the top
- [ ] **Architecture diagram** (crates and data flow)
- [ ] **Design decisions** section, from `docs/decisions.md`
- [ ] Install instructions and a link to the web playground
- [ ] Badges: CI, license, release
- [ ] Resume bullet written as outcomes, with real numbers from benchmarks
- [ ] Example `.cas` files showing source, render blocks, and the PDF result
- [ ] Use it for your own coursework and mention that

---

## Scope guard

- Finish Phases 0 to 3 before anything else. The documenter (Phase 1) is the product; everything after it improves it.
- Pick **one or two** items from Phase 5 rather than all of them.
- The AI layer (Phase 8) is optional and should never be the default rendering path.
- If a phase drags on, cut items, not quality.
