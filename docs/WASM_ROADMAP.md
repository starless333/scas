# Browser IDE roadmap

## Goal

Let someone open SCAS in a browser, edit a .scas document, run it, inspect
diagnostics, preview mathematics, use the REPL and plots, and keep or download
their work. The first public build should run as a static site, so GitHub
Pages can host it without an application server. The native IDE remains
available.

Current status (2026-10-09): a local static browser build runs the shared SCAS
core in a Web Worker and provides an egui source editor, clickable diagnostics,
output, MathJax mathematical preview, graphing, examples, import/download,
draft recovery through localStorage, and URL-hash sharing. Build with
`bash scripts/build-web.sh`, then serve `web/` over HTTP. The Pages workflow
builds and uploads this directory, but public deployment has not been verified.
The browser interface is a separate `WebApp` that reuses the language core and
math preview; it does not yet reuse the entire native `ScasApp` or offer its
terminal, persistent REPL, project tree, or PDF pipeline.

SCAS already has a reusable language library (src/lib.rs) and an egui UI.
The current GUI is a native executable, however. It reads and writes project
files with std::fs, starts worker threads, invokes latex/dvisvgm/rsvg-convert and pdflatex,
loads themes and fonts from the host, and uses native window controls. Those
boundaries determine the port.

| Capability | Native implementation today | Browser implementation |
| --- | --- | --- |
| Editor files | Workspace paths and std::fs | In-memory document, browser import/download, localStorage draft |
| Execution | Shared SCAS library | Same library compiled to Wasm with the existing resource limits |
| Live analysis and graph sampling | std::thread and channels | Web Worker messages with generation IDs; in-flight cancellation is pending |
| Math preview | External LaTeX tools, then native fallback | MathJax 4 typesets document fragments in the browser; TeX download remains available |
| PDF export | pdflatex process | Defer until a browser compiler or optional service is evaluated |
| Themes and fonts | Bundled files plus host file loading | Embedded dark defaults; theme import is future work |
| Window chrome | Native viewport actions | Browser-safe controls; no minimize/maximize/OS drag actions |

## Milestones

### 1. Prove the language core on Wasm

- Add the wasm32-unknown-unknown target in development and CI, then get
  cargo check --lib --target wasm32-unknown-unknown passing. This target is
  installed locally and checked in the Pages workflow.
- Isolate the file-writing branch of src/plot.rs and the file-reading helper
  of src/latex.rs behind native adapters or target gates. Keep parsing,
  evaluation, document assembly, SVG generation, and plot sampling in the
  shared library.
- Add a small browser-facing API for execute, build-document, and sample-plot
  operations that accepts source text and returns structured results and
  diagnostics. Keep the existing deterministic limits on every call.
- Verify exact arithmetic and the MIT example produce the same results on
  native and Wasm.

**Exit gate:** a browser test can evaluate SCAS and build a document from text
without host files or subprocesses.

**Status:** implemented and tested with the shared worker-request API. Exact
arithmetic and MIT output still need automated cross-target parity fixtures.

### 2. Separate the GUI from native services

- Move ScasApp and reusable editor components out of the native-only
  src/bin/scas-gui.rs entry point. Define a small platform interface for
  open/save, persistence, background jobs, export, and viewport actions.
- Keep a native adapter for the existing filesystem, threads, and LaTeX
  pipeline. Add a web adapter for imported files, browser storage, downloads,
  and asynchronous work.
- Use target-specific Cargo dependencies/features so the web build does not
  pull in the native eframe X11/Wayland features or OS-only APIs.
- Add an eframe::WebRunner entry point and a canvas HTML shell. eframe
  0.28 uses WebRunner for the web; build the Wasm module with
  wasm32-unknown-unknown and generate JavaScript bindings with
  wasm-bindgen.

**Exit gate:** one SCAS UI codebase opens on desktop and in a browser, with
native-only commands absent from the web interface.

**Status:** WebRunner and a browser-safe interface are implemented. Full
`ScasApp` extraction and platform-adapter unification are not complete.

### 3. Ship a useful browser workspace

- Start with one in-memory workspace: editor, diagnostics, Run, REPL, graphing,
  and a scrollable structured math preview. Let users import .scas files,
  download their source and generated .tex/.svg, and recover drafts from
  IndexedDB. Add project-folder import only after single-file workflows are
  sound.
- Keep local Computer Modern SVG-backed rendering on desktop. The browser's
  MathJax preview is the rendering path; a browser PDF compiler
  needs its own size, startup-time, and licensing review.
- Move expensive analysis and graph sampling into a Web Worker. Preserve the
  GUI's generation IDs so stale results cannot replace newer edits, and use
  the existing step/precision budgets to bound work.
- Test reload recovery, import/export round trips, large documents, Unicode
  input, errors, and narrow screens. Compare native and browser output on
  representative fixtures.

**Exit gate:** a visitor can complete and retain a SCAS document in the
browser without installing a toolchain.

**Status:** single-file source editing, execution, preview, graphing, and
download work locally. Browser REPL, durable IndexedDB storage, cancellation,
and broader browser smoke tests remain open.

### 4. Publish a static preview

- Build an HTML, JavaScript, Wasm, and asset directory with relative URLs that
  work under a repository path such as /scas/.
- Add a CI job that checks native and Wasm builds, runs browser smoke tests,
  and uploads the static directory. Deploy with GitHub Pages Actions after
  the preview passes review. Keep the current documentation available at a
  stable route and link the browser IDE from it.
- Measure first-load bytes and startup time. Compress Wasm, cache immutable
  assets, and show loading/error states while the module initializes.

**Exit gate:** the public URL loads on desktop and mobile browsers, accepts a
document, runs it, and restores a saved draft after refresh.

**Status:** a Pages workflow exists; public URL, startup metrics, and
production smoke checks remain unverified.

## First implementation slice

Start with Milestone 1 and a minimal WebRunner canvas that can evaluate the
starter document. This will reveal target-specific dependency failures before
the GUI is split. The first browser release does not need PDF export or direct
access to the visitor's filesystem; .scas, .tex, and .svg downloads cover
the essential workflow.

## References

- [eframe web setup and WebRunner](https://github.com/emilk/egui/blob/main/crates/eframe/src/lib.rs)
- [eframe 0.28 WebRunner change](https://github.com/emilk/egui/blob/main/crates/eframe/CHANGELOG.md)
- [GitHub Pages deployment with Actions](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)
