# SCAS notebook design

## Product thesis

SCAS should remain a text-file language and gain a notebook **interaction
mode**, not a second notebook-only file format.

A `.scas` file is the canonical source. The same file must continue to run in
the CLI, open in third-party editors through the LSP, build into TeX or PDF,
and work in both SCAS IDEs. Notebook mode groups that source into executable
regions and places structured results beside it. It does not replace the
source with JSON, store opaque execution order in the document, or make the
GUI the only way to reproduce a result.

The product should feel like an **auditable mathematical workbench**:

- explore interactively like a notebook;
- preserve exact symbolic meaning like a CAS;
- rerun deterministically like a program;
- explain and verify results like an executable document;
- publish the same source as a technical report.

That combination is more distinctive and useful than cloning Jupyter's file
format or adding a larger collection of disconnected symbolic functions.

## What the current project already provides

Notebook mode should reuse, rather than duplicate, these existing systems:

- `.scas` source, modules, the standard library, and host-independent I/O;
- `///` document prose and interpolation;
- `\[ ... \]`, `\{ ... \}`, `=>`, derivation traces, and structured document
  fragments;
- exact arithmetic, explicit `evalf`, symbolic assumptions, and verification;
- source spans, diagnostics, completion, hover evaluation, definitions,
  references, and rename;
- persistent REPL environments;
- graph sampling and SVG export;
- native background workers and the browser Web Worker protocol;
- live preview with last-known-good output;
- deterministic parser, evaluation, precision, recursion, and size limits;
- TeX/PDF export, themes, project files, browser draft recovery, and shareable
  source links.

The notebook must be a shared library feature consumed by the CLI, native IDE,
and browser IDE. It must not be implemented as unrelated cell logic in each
frontend.

## Source and cell model

### Canonical representation

- A notebook is an ordinary UTF-8 `.scas` file.
- Output is derived state and is not inserted into source automatically.
- Whole-file execution remains the definition of a reproducible run.
- The CLI remains able to execute every notebook without notebook-specific
  runtime semantics.
- TeX/PDF builds continue to follow source order.

### Cell boundaries

Notebook mode should recognize two kinds of boundaries:

1. **Semantic boundaries** already present in SCAS: prose paragraphs, render
   lines, render blocks, declarations, and top-level statement groups.
2. **Optional editor boundaries** written as `// %%`. This is an ordinary
   comment to the language, so older versions and the CLI safely ignore it.

Example:

```scas
/// # Damped oscillator
/// Compare the exact solution with measured samples.

// %% parameters
let gamma = 1 / 5;
let omega = 3;

// %% symbolic solution
let response(t) = exp(-gamma*t) * cos(omega*t);
\[ derivative(response(t), t) => \]

// %% verification
assert(subs(response(t), t, 0) == 1);

// %% visualization
plot(response(t), t, 0, 10);
```

The label after `// %%` is navigation text, not an identifier or execution
dependency. A future stable cell ID may be stored in a sidecar, but users
should not have to maintain generated IDs in normal source.

### Cell presentation

Notebook mode may present the following views without inventing new language
constructs:

- **Prose:** consecutive `///` lines, rendered as documentation.
- **Code:** ordinary declarations, statements, functions, and control flow.
- **Math:** render lines and render blocks with typeset results.
- **Verification:** `assert`, `verify`, and `verify_chain`, with an explicit
  pass, fail, or unproved state.
- **Visualization:** plot expressions with the interactive graph and an
  exportable SVG result.
- **Input/output:** prompts, printed text, diagnostics, and imported data.

Users must always be able to reveal and edit the underlying `.scas` text.

## Execution model

### Two modes, one meaning

Notebook interaction needs both convenience and reproducibility:

- **Run cell** evaluates the selected cell in a persistent session.
- **Run above** resets the session and evaluates source through the selected
  cell in canonical order.
- **Run all** resets the session and evaluates the complete file from the
  beginning.
- **Build document** performs the same clean, ordered evaluation used by the
  CLI document builder.

Only a clean `Run all` or document build may mark a notebook reproducible.
Interactive execution is allowed to be stateful, but the interface must never
pretend that an out-of-order session is equivalent to a clean run.

### State model

Every cell should expose one of these redundant text-and-shape states:

- **not run**;
- **queued**;
- **running**;
- **current** — result matches the source and upstream state;
- **stale** — source or an upstream dependency changed;
- **failed** — evaluation or verification failed;
- **blocked** — an earlier required cell failed or input is required;
- **cancelled**.

Editing a cell marks it and all later state-dependent cells stale. Initially,
SCAS can conservatively mark every following cell stale. A dependency graph
may make this more precise later, but it must never under-report staleness.

### Kernel/session behavior

- Restart creates a new symbol table, interpreter, output buffer, I/O state,
  assumptions, precision configuration, and random seed.
- Native and browser sessions implement the same request/response protocol.
- Long work is cancellable. Cancellation must terminate the active evaluation,
  not merely ignore its eventual result.
- A generation ID prevents an older result from replacing a newer run.
- Resource limits are displayed with an actionable diagnostic when exhausted.
- A cell waiting for input visibly pauses rather than blocking the UI thread.
- A kernel crash preserves source and previously rendered output, reports the
  failure, and offers Restart and Run All.

### Reproducibility record

A successful clean run should be able to produce a small manifest containing:

- SCAS version and target;
- source hash;
- imported module hashes;
- precision and resource-limit settings;
- random seed when randomness is introduced;
- input data names and content hashes;
- run timestamp as optional metadata, never computational input;
- verification summary.

The manifest can be embedded in an exported report or written beside it. The
source must remain executable without the manifest.

## Structured output protocol

The current string-oriented execution paths should evolve into a shared
ordered stream of typed output events. A result should carry:

- cell ID or source span;
- generation ID;
- sequence number;
- output kind;
- plain-text fallback;
- structured payload;
- source and diagnostic spans;
- execution duration and optional resource usage;
- stale/current status.

Initial output kinds:

- plain text and standard output;
- scalar or symbolic value;
- LaTeX plus structured-math fallback;
- derivation or verification trace;
- matrix or vector;
- table;
- plot specification and SVG;
- diagnostic;
- input request;
- file/download artifact.

Every rich output must have a text representation for the CLI and
accessibility. Frontends render the same event differently; they do not rerun
the mathematics to manufacture their own output.

## Output persistence

- Default notebook output is ephemeral and recomputable.
- Draft recovery may cache output locally, keyed by SCAS version, source hash,
  upstream-state hash, precision, and imported-data hashes.
- Cached output is never treated as verified after a key changes.
- Cache files belong in ignored application storage or an ignored sidecar,
  not inside `.scas`.
- Exported HTML/PDF is a publication artifact, not the source of truth.
- A future portable snapshot may bundle source, data, manifest, and rendered
  output, but importing it must recover the original `.scas` file exactly.

## Notebook interface

Notebook mode should coexist with the current document editor rather than
replace it.

### Required interactions

- Toggle between Source, Notebook, and split Source/Preview views.
- Run cell, Run above, Run all, Interrupt, and Restart kernel.
- Move focus between cells without changing source order accidentally.
- Insert a code, prose, rendered-math, verification, or plot template.
- Collapse output without hiding failure or stale state.
- Jump from an output, diagnostic, or preview fragment to its source span.
- Copy a result as plain text, SCAS source, LaTeX, CSV, or SVG when applicable.
- Clear outputs without changing source.
- Search source, prose, symbols, and textual output.
- Show execution duration only when useful; do not turn the document into a
  dashboard.

### Keyboard model

- Standard editor shortcuts continue to work inside source.
- `Ctrl/Cmd+Enter`: run cell without moving.
- `Shift+Enter`: run cell and move to the next cell.
- `Alt+Enter`: run cell and insert a boundary below.
- Escape returns focus from rich output to source navigation.
- All output controls are keyboard reachable and have visible focus.

Shortcuts must be configurable and must not override essential platform text
editing behavior.

### Trust and safety

Opening source must never execute it automatically. Imported or downloaded
notebooks begin untrusted and require an explicit run action. Before native
execution, SCAS should disclose capabilities with side effects, including file
writes and imported modules. Browser execution remains constrained to virtual
I/O and explicit downloads.

## What would make SCAS genuinely useful beyond homework

The largest gap is not another page of symbolic builtins. It is the ability to
take a real technical question from data and assumptions through computation,
verification, explanation, and publication.

### 1. Quantities, units, and uncertainty

Engineering and experimental work needs dimensional quantities rather than
unlabeled numbers.

Goals:

- first-class units and dimension checking;
- exact conversion factors where possible;
- affine units such as temperatures handled safely;
- significant figures and measurement uncertainty;
- propagation of uncertainty through supported operations;
- unit-aware plotting, tables, solving, and LaTeX output;
- diagnostics for dimensionally invalid expressions.

This would turn SCAS verification from “the algebra is equal” into “the result
is mathematically and dimensionally coherent.”

### 2. Tabular data and statistics

`read_csv` and arrays are a start, but professional analysis needs a coherent
table value and reproducible transformations.

Goals:

- named columns, schemas, missing values, filtering, sorting, grouping, joins,
  and aggregation;
- clean CSV/JSON import diagnostics with row and column context;
- descriptive statistics, quantiles, covariance, correlation, and common
  distributions;
- linear and nonlinear regression with residuals and confidence information;
- table previews that remain usable with large inputs;
- publication-quality tables and plots;
- streaming or bounded import so large data does not freeze the IDE.

### 3. Numerical problem solving

Symbolic methods should fall back explicitly to reliable numerical tools when
an exact solution is unavailable.

Goals:

- bracketed and open root-finding with convergence reports;
- systems of nonlinear equations;
- constrained and unconstrained optimization;
- numerical integration and ODE initial-value solvers;
- interpolation and curve fitting;
- sparse matrices and larger numerical linear algebra;
- interval or residual-based result validation where practical;
- progress, cancellation, iteration limits, and precision controls.

Every approximate result should report method, tolerance, iterations, and a
residual or error estimate rather than presenting a decimal as unquestioned
truth.

### 4. Reusable packages and environments

Real users need to build on one another's work.

Goals:

- a documented module/package layout;
- semantic versions and a lockfile;
- local/path dependencies before a public registry;
- generated API documentation from SCAS modules;
- namespaced imports and clear public/private symbols;
- package tests runnable by the CLI and CI;
- reproducible environments included in notebook manifests;
- curated domain libraries for statistics, numerical methods, physics, and
  engineering.

### 5. Interoperability

SCAS does not need to replace the scientific ecosystem. It needs clean
boundaries to it.

Goals:

- stable CSV and JSON interchange first;
- Matrix Market and common numerical text formats;
- export of plots as SVG and data as CSV;
- HTML report export with accessible MathML or a robust math fallback;
- a documented Rust library API;
- later evaluation of Arrow, Python, or Jupyter interoperability based on real
  user demand rather than embedding another runtime by default.

### 6. Explainability and provenance

SCAS already has derivation steps and checked claims; this should become a
first-class differentiator.

Goals:

- stable, named transformation steps rather than presentation-only strings;
- expandable “why this step is valid” explanations;
- assumptions attached to results and visible in output;
- provenance from a result back to source, inputs, rules, and imported data;
- proof/check summaries in exported reports;
- deterministic traces suitable for regression tests;
- explicit “unproved” distinct from false.

### 7. A flagship workflow

The project needs several polished, end-to-end examples that solve credible
problems rather than only demonstrating individual functions:

- an engineering calculation with units, tolerances, verification, and a
  publishable report;
- a small real dataset with cleaning, regression, residual analysis, and plots;
- a symbolic-to-numeric model where an exact derivation is validated against
  numerical results;
- a reproducible linear-algebra or control-system analysis with imported data;
- a package authored in SCAS, tested, imported, and used by a notebook.

These examples should be automated fixtures. A release is not complete if the
flagship workflows fail to run cleanly on the CLI, native IDE, and browser
within their documented platform limits.

## Implementation roadmap

### Phase 0 — decisions and shared contracts

- [ ] Confirm `.scas` as the only canonical notebook source format.
- [ ] Reserve `// %%` as an optional editor boundary that remains a comment.
- [ ] Specify stable cell/source-span identifiers.
- [ ] Specify typed output events and plain-text fallbacks.
- [ ] Specify session reset, stale-state, cancellation, and input semantics.
- [ ] Define notebook trust behavior for native and browser hosts.
- [ ] Add representative notebook fixtures before building the UI.

**Exit goal:** the source model, event protocol, and execution semantics can be
implemented without frontend-specific exceptions.

### Phase 1 — reproducible notebook kernel

- [ ] Extract a shared `NotebookSession` around the symbol table, interpreter,
  assumptions, precision, I/O, and resource limits.
- [ ] Parse source into conservative cells while preserving exact byte spans.
- [ ] Implement Run cell, Run above, Run all, Interrupt, and Restart.
- [ ] Mark downstream cells stale after edits or state changes.
- [ ] Stream typed text, math, diagnostic, verification, and input events.
- [ ] Preserve generation IDs and prevent stale result replacement.
- [ ] Add true worker cancellation in the browser.
- [ ] Generate a clean-run reproducibility manifest.
- [ ] Provide a CLI command that executes notebook regions and emits JSON
  events for testing and integrations.

**Exit goal:** the same notebook fixture produces equivalent ordered events in
native tests, the CLI, and the browser worker.

### Phase 2 — notebook interface

- [ ] Add Source, Notebook, and split views without duplicating editor state.
- [ ] Render prose, math, derivations, diagnostics, verification, and plots
  from shared output events.
- [ ] Implement cell navigation and execution shortcuts.
- [ ] Add visible not-run, running, current, stale, failed, blocked, and
  cancelled states.
- [ ] Add source/output bidirectional navigation.
- [ ] Add output copy/export and Clear Outputs.
- [ ] Preserve source and recover gracefully after worker/kernel failure.
- [ ] Make all controls accessible by keyboard and screen reader.
- [ ] Adapt notebook mode for narrow browser windows without hiding source.

**Exit goal:** a new user can open an example, change parameters, run selected
work, identify stale results, perform a clean rerun, and export a document.

### Phase 3 — durable projects and publishing

- [ ] Add autosave and crash recovery for source, not only cached output.
- [ ] Store output cache separately and invalidate it by content hashes.
- [ ] Hash imports and input data in the run manifest.
- [ ] Add self-contained accessible HTML report export.
- [ ] Embed reproducibility and verification summaries in HTML/PDF exports.
- [ ] Add project-relative data files and portable project bundles.
- [ ] Add an explicit trust prompt for imported bundles.
- [ ] Test import/export and recovery across version upgrades.

**Exit goal:** a user can send a project to another machine, rerun it, and
distinguish a reproduced result from a cached snapshot.

### Phase 4 — professional computation

- [ ] Introduce first-class quantities and dimensional analysis.
- [ ] Add uncertainty-aware quantities and formatting.
- [ ] Introduce a typed table value with missing-data semantics.
- [ ] Add core descriptive statistics and regression.
- [ ] Add numerical root-finding, optimization, quadrature, and ODE solving
  with convergence metadata.
- [ ] Add package manifests, lockfiles, module documentation, and package tests.
- [ ] Build the flagship engineering and data-analysis notebooks.

**Exit goal:** at least two non-homework workflows can be reproduced, audited,
and published without leaving SCAS.

### Phase 5 — ecosystem and interoperability

- [ ] Stabilize the Rust embedding API.
- [ ] Add standard numerical interchange formats justified by flagship users.
- [ ] Publish curated SCAS packages and versioned examples.
- [ ] Evaluate Jupyter protocol support as an adapter over `NotebookSession`,
  not as SCAS's canonical storage format.
- [ ] Evaluate Python or Arrow bridges only after measuring demand and binary
  cost.
- [ ] Add a public notebook gallery that runs every example in CI.

**Exit goal:** users can adopt SCAS within an existing workflow instead of
having to replace the rest of their toolchain.

## Quality checklists

### Correctness and reproducibility

- [ ] Run All always begins from a clean session.
- [ ] Cell execution never changes whole-file language semantics.
- [ ] An upstream edit cannot leave a downstream result marked current.
- [ ] Exact and approximate results remain visibly distinct.
- [ ] Approximate algorithms report convergence and residual information.
- [ ] Assumptions and precision affecting a result are inspectable.
- [ ] Cached output is never confused with freshly executed output.
- [ ] Clean native, CLI, and browser runs agree on shared fixtures.
- [ ] Verification failures make document and CI builds fail.

### Reliability and performance

- [ ] The UI remains responsive during parsing, evaluation, plotting, and
  rendering.
- [ ] Interrupt stops work within a documented bound.
- [ ] Rapid reruns cannot display an older generation as current.
- [ ] Large output is truncated or virtualized with access to the full export.
- [ ] Large tables and matrices do not create unbounded widget counts.
- [ ] Kernel failure cannot lose unsaved source.
- [ ] Browser memory use is measured for the main and worker Wasm instances.
- [ ] Startup, first result, Run All, and export times have regression budgets.

### Security and privacy

- [ ] Opening a file never executes it.
- [ ] Imported notebooks are visibly untrusted until the user runs them.
- [ ] Native file writes and external-process capabilities are disclosed.
- [ ] Browser notebooks cannot escape virtual I/O through notebook features.
- [ ] HTML and rich output are sanitized before rendering.
- [ ] Resource limits apply equally to cell and whole-file execution.
- [ ] Shared links contain no local files or inputs unless explicitly included.

### Accessibility and usability

- [ ] Every action is keyboard accessible.
- [ ] Status is communicated by text/shape as well as color.
- [ ] Screen readers receive cell labels, execution state, diagnostics, and
  text alternatives for math and plots.
- [ ] Focus returns predictably after execution or output interaction.
- [ ] Source remains usable at 200% zoom and in narrow layouts.
- [ ] Reduced-motion preferences disable nonessential transitions.
- [ ] Copy/export formats are named explicitly.

### Documentation and adoption

- [ ] Getting Started includes a five-minute notebook workflow.
- [ ] Every notebook action documents its equivalent source/CLI behavior.
- [ ] Examples progress from exploration to a clean reproducible run.
- [ ] Error messages state both the problem and a recovery action.
- [ ] The public gallery contains credible engineering, research, and data
  examples rather than only feature demonstrations.
- [ ] Notebook compatibility and output-cache policy are versioned.

## Non-goals for the first notebook release

- A JSON notebook as the primary source format.
- Real-time multi-user editing.
- Arbitrary HTML or JavaScript output from user code.
- Automatic execution when a file opens.
- A public package registry before local packages and lockfiles are sound.
- Python/R runtime embedding in the initial Wasm download.
- Perfect dependency analysis before conservative stale-state handling works.
- Saving output as a substitute for reproducible execution.

## Success criteria

Notebook mode succeeds when:

1. A `.scas` file is equally trustworthy in the CLI, an ordinary editor, and
   notebook mode.
2. Interactive execution is fast, but a clean rerun is always one action away
   and clearly distinguished from historical state.
3. Rich outputs come from shared typed events rather than frontend-specific
   evaluation.
4. A user can move from exploration to a verified HTML/PDF report without
   copying results between tools.
5. SCAS demonstrates at least two credible workflows involving real data,
   units or uncertainty, numerical methods, reusable modules, and verification.
6. The notebook strengthens SCAS's text-first language instead of dividing the
   project into incompatible document and notebook products.
