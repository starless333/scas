# Product

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

## Users

Developers and technical students authoring, running, and reviewing SCAS documents in a desktop development workflow.

## Product Purpose

SCAS is an exact-arithmetic scripting language and symbolic computer algebra system. Its desktop IDE lets users write executable mathematical documents, inspect diagnostics and program output, and preview typeset mathematics without leaving the authoring environment. Success means the editor feels immediately trustworthy to someone already fluent in professional code editors.

## Positioning

SCAS combines a programming-language workbench with an explicit-markup mathematical documenter: source, evaluation, diagnostics, and live typeset output remain connected in one workspace.

## Operating Context

Users work with `.scas` files and project folders, author `///` prose and explicit render blocks, run programs, resolve parser and document diagnostics, and compare source against LaTeX-backed output. VS Code and Zed are the confirmed interaction and quality references.

## Capabilities and Constraints

- Native Rust desktop application built with `eframe`/`egui`.
- Existing file tree, text editor, syntax highlighting, diagnostics, execution output, and live math preview must remain functional.
- Theme files use TOML and user-loaded themes must remain supported.
- The interface should use familiar desktop IDE structure and dense, predictable controls.

## Brand Commitments

- Product name: SCAS.
- The GUI should read as an industry-standard developer tool in the family of VS Code and Zed, not as a marketing surface or novelty editor.

## Evidence on Hand

- Product and language documentation under `docs/`.
- IDE roadmap and confirmed feature scope in `CAS_IDE.md`.
- Existing GUI implementation in `src/bin/scas-gui.rs` and theme schema in `src/bin/scas_gui/theme.rs`.
- Existing bundled themes under `themes/`.

## Product Principles

- Keep source, diagnostics, execution, and rendered mathematics in one coherent workspace.
- Prefer familiar editor conventions so users can focus on mathematical work.
- Make important state—active file, dirty state, run state, diagnostics, and preview availability—legible at a glance.
- Preserve a fast local workflow and user-extensible configuration.

## Accessibility & Inclusion

Maintain both dark and light theme families, clear focus and selected states, readable contrast, scalable editor type, and redundant text or shape cues for status where practical.
