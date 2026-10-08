---
name: SCAS Desktop IDE
description: A VS Code-inspired, editor-first native workspace for mathematical programs.
colors:
  workspace: "#181818"
  panel: "#252526"
  editor: "#1f1f1f"
  title-bar: "#3c3c3c"
  activity-bar: "#333333"
  text: "#cccccc"
  muted: "#9d9d9d"
  accent: "#007acc"
  accent-contrast: "#ffffff"
  selection: "#264f78"
  syntax-plain: "#d4d4d4"
  syntax-keyword: "#c586c0"
  syntax-number: "#b5cea8"
  syntax-string: "#ce9178"
  syntax-comment: "#6a9955"
  syntax-doc-comment: "#608b4e"
  syntax-identifier: "#9cdcfe"
  syntax-operator: "#d4d4d4"
  syntax-delimiter: "#ffd700"
  syntax-render-marker: "#4ec9b0"
typography:
  heading:
    fontFamily: "Segoe WPC (Windows), SF Pro Text (macOS), Ubuntu (Linux), sans-serif"
    fontSize: "16px"
    fontWeight: 600
  body:
    fontFamily: "Segoe WPC (Windows), SF Pro Text (macOS), Ubuntu (Linux), sans-serif"
    fontSize: "13px"
    fontWeight: 400
  control:
    fontFamily: "Segoe WPC (Windows), SF Pro Text (macOS), Ubuntu (Linux), sans-serif"
    fontSize: "12px"
    fontWeight: 400
  label:
    fontFamily: "Segoe WPC (Windows), SF Pro Text (macOS), Ubuntu (Linux), sans-serif"
    fontSize: "11px"
    fontWeight: 600
  code:
    fontFamily: "JetBrains Mono, ui-monospace, monospace"
    fontSize: "27px"
    fontWeight: 400
rounded:
  none: "0px"
  control: "2px"
  window: "4px"
spacing:
  xxs: "2px"
  xs: "4px"
  sm: "6px"
  md: "8px"
  lg: "10px"
  xl: "12px"
  preview-inset: "18px"
components:
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-contrast}"
    typography: "{typography.control}"
    rounded: "{rounded.control}"
    padding: "3px 7px"
    height: "24px"
  button-neutral:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.text}"
    typography: "{typography.control}"
    rounded: "{rounded.control}"
    padding: "3px 7px"
    height: "24px"
  command-bar:
    backgroundColor: "{colors.title-bar}"
    textColor: "{colors.text}"
    typography: "{typography.control}"
    rounded: "{rounded.none}"
    padding: "5px 8px"
    height: "38px"
  activity-button:
    backgroundColor: "{colors.activity-bar}"
    textColor: "{colors.muted}"
    rounded: "{rounded.control}"
    size: "40px 38px"
  editor-tab-active:
    backgroundColor: "{colors.editor}"
    textColor: "{colors.text}"
    typography: "{typography.control}"
    rounded: "{rounded.none}"
    padding: "5px 10px"
  status-bar:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-contrast}"
    typography: "{typography.label}"
    rounded: "{rounded.none}"
    padding: "2px 8px"
    height: "26px"
---

# Design System: SCAS Desktop IDE

## Overview

**Creative North Star: "The Mathematical Workbench"**

SCAS is a dense, native Operate-mode desktop IDE. The supplied VS Code dark reference anchors its window chrome and panel hierarchy; restrained spacing and sparse accent use keep attention on the source editor. A borderless, maximized window opens with a custom title bar, Explorer, an active document tab, breadcrumbs, and a status bar. The visual system is flat and theme-led.

The editor owns startup. Mathematical preview and the bottom output/problems dock are available on demand, while running a document opens output automatically. When opened, preview stays adjacent to source and retains the last good result during editing errors. Twelve bundled themes share the semantic TOML contract, with Dark Modern as the default and external theme loading supported.

**Key Characteristics:**

- Dense, flat panels divided by quiet one-pixel structure.
- Compact 11–13px interface chrome around a user-scalable 27px starting code surface.
- An editor-first layout with collapsible and resizable supporting panes.
- One theme accent reserved for selection, active state, live state, and status.
- Custom vector icons drawn in a consistent 1.6px stroke language.
- A familiar title bar, project Quick Open, Explorer, tabs, breadcrumbs, and status bar.

## Colors

The default Dark Modern palette uses separate title bar, activity rail, Explorer, and editor surfaces, soft gray text, and editor-blue accent. Its TOML file defines nine UI roles and ten syntax roles. The title-bar and activity-bar roles are optional for older or external themes and fall back to panel and background respectively.

### Primary

- **Workbench Blue:** The interactive accent. It marks Run, selected state, live preview state, focus, and the status bar.

### Tertiary

- **Syntax Spectrum:** Plain text, keywords, numbers, strings, comments, documentation, identifiers, operators, delimiters, and render markers receive distinct semantic roles. Themes may change hues, but never collapse the role schema.

### Neutral

- **Workspace Charcoal:** The deepest shell layer behind the workspace.
- **Title Bar Gray:** The custom draggable title bar and centered project Quick Open control.
- **Activity Bar Gray:** The fixed left destination rail.
- **Panel Charcoal:** Explorer, tab strip, and optional preview.
- **Editor Charcoal:** The source editor, active tab, and output dock.
- **Soft Foreground:** Primary labels and interface content; it avoids stark white glare.
- **Muted Steel:** Secondary labels, inactive icons, tree glyphs, and separators at reduced opacity.
- **Accent Contrast:** Derived white or near-black text that remains legible on the selected theme's accent.
- **Selection Blue:** Selected and hovered control fill without introducing a second accent.

### Named Rules

**The One Accent Rule.** Accent color communicates action or state; it is not ambient decoration.

**The Semantic Theme Rule.** Components consume named UI and syntax roles, never theme-specific color literals.

## Typography

**Interface Font:** Segoe WPC on Windows, SF Pro Text on macOS, and Ubuntu on Linux, with egui's proportional font as the last fallback

**Code Font:** JetBrains Mono, bundled under the SIL Open Font License, with egui's monospace font as the fallback

**Character:** Typography is deliberately utilitarian and compact. Weight, case, and semantic color create hierarchy within a narrow 11–16px range so the workspace stays information-dense without becoming noisy.

### Hierarchy

- **Heading** (semibold, 16px): Dialog and rare high-level headings.
- **Body** (regular, 13px): Standard interface copy, tree entries, output, and supporting content.
- **Control** (regular, 12px): Menus, buttons, active file tabs, and compact actions.
- **Label** (semibold, 11px): Uppercase panel titles, OUTPUT/PROBLEMS tabs, state, metadata, and the status bar.
- **Code** (regular, 27px startup default): Editable SCAS source in JetBrains Mono. Users may scale or replace this font without changing the chrome hierarchy.

### Named Rules

**The Chrome Ceiling Rule.** Routine interface chrome stays between 11px and 13px; the editor font is independently scalable and starts at 27px.

## Layout

The borderless window starts maximized. A 38px custom title bar contains VS Code-style menus, a centered project control that opens Quick Open, panel toggles, and window controls. Below it are a fixed 46px activity rail and an open-by-default Explorer at 300px, resizable from 200–420px. The source editor takes the remaining width on startup. Preview and output start closed; the right preview opens at about 46% of usable width, bounded by editor space and a 900px maximum, and the bottom dock is resizable. A 26px accent status bar anchors the bottom.

Spacing follows a compact 2/4/6/8/10/12px rhythm. Panel headers use 8–10px horizontal insets and 5–7px vertical insets. Explorer hierarchy advances by 12px per depth level, with explicit disclosure, folder, and file cues. The rendered-document inset uses a more generous 18px to distinguish the typeset artifact from surrounding chrome.

The active document tab uses editor fill and a compact SC file mark. Its filename carries the dirty dot. A 23px breadcrumb row gives the current relative file path above the editor. The optional dock exposes OUTPUT and PROBLEMS with a live problem count. Quick Open is a compact top-centered file search over workspace files.

**The Editor-First Rule.** Source receives the available workspace at launch; supporting panes open only when requested or needed by an action.

## Elevation & Depth

The primary workspace has no shadows. Depth is structural: adjacent charcoal tones, one-pixel separators, selection fills, and the accent underline establish containment. Shadows are reserved for transient native windows and popups: windows use a compact ambient shadow and popups a slightly broader one so temporary layers read above the editor without making persistent panels float.

### Shadow Vocabulary

- **Window Ambient** (`0 6px 18px rgba(0,0,0,0.31)`): Native settings and dialog windows only.
- **Popup Ambient** (`0 8px 24px 2px rgba(0,0,0,0.37)`): Menus, completion lists, and transient popups only.

### Named Rules

**The Flat Workspace Rule.** Persistent IDE panels never float; use tonal layers and separators, not card shadows.

## Shapes

The form language is rectilinear and compact. Controls use a restrained 2px radius, transient windows use 4px, and persistent panels and tabs remain square. One-pixel strokes define focus and boundaries. Activity icons occupy a 20px drawing box and share a 1.6px primary stroke; tree folder and file cues use compatible 1.0–1.2px strokes. Do not mix in filled emoji, icon fonts, or an unrelated outline family.

## Components

### Buttons

- **Shape:** Compact rectangle with a restrained 2px radius and 24px interaction height.
- **Primary:** Accent fill, automatic high-contrast text, and 7px by 3px internal padding; reserved for Run and similarly decisive actions.
- **Hover / Focus:** Hover adopts the semantic selection fill; keyboard focus receives a one-pixel accent outline. State transitions use the shared 160ms native animation time.
- **Neutral:** Panel fill, primary text, and a quiet one-pixel muted stroke.

### Inputs / Fields

- **Style:** Editor-toned field, one-pixel muted border, 2px corners, compact body type.
- **Focus:** Accent border with no glow and no layout shift.
- **Error / Disabled:** Errors use the theme's syntax operator role and explanatory text; disabled controls retain structure at reduced contrast.

### Navigation

- **Title bar:** A 38px custom strip replaces operating-system decorations. It contains the SCAS mark, File/View/Run and other menus according to width, a centered project Quick Open control, panel toggles, and minimize/maximize/close controls. The open strip drags the window; double-clicking it toggles maximization.
- **Activity rail:** A fixed 46px rail provides Explorer, Quick Open, REPL, Graphing, and Settings. Preview and Problems stay accessible through the title bar, View menu, editor actions, and output dock without duplicating destinations in the rail. Selection uses the theme's title-bar tint and indicator.
- **Explorer:** Open by default at 300px, compact, resizable, and hierarchical. Its root and folder rows disclose children; the header includes new-file, refresh, and collapse actions.
- **Quick Open:** File and Go menus, the rail, the centered project control, and Ctrl+P open a top-centered search over workspace files. Enter opens the first match; Escape or a click outside dismisses it.

### Editor Tab

The active document tab uses editor fill, an SC file mark, a shortened filename with a dirty dot when needed, and a close action. REPL and Graphing appear as workspace tabs when active. A breadcrumb row below the tabs displays the current file location. Editor actions include preview, run, math-symbol insertion, and snippets.

### Live Typeset Preview

Preview starts closed. The title bar, View menu, or editor action opens it beside source. Its resizable panel shows DOCUMENT PREVIEW, LIVE/UPDATING, Computer Modern, Copy TeX, and rendered mathematics; editing errors preserve the last good preview. LaTeX source remains in a subordinate disclosure.

### Output Dock

OUTPUT and PROBLEMS share a resizable bottom dock that starts closed. Running a document opens it to OUTPUT, while the View menu opens diagnostics. Ctrl+J, the View menu, and the title-bar panel control toggle the dock. Output is selectable and copyable, and diagnostics retain line and column context.

### Live Math Entry

Enter evaluates the current REPL input symbolically. Ctrl+Enter executes a hidden `evalf(...)` wrapper for immediate numerical evaluation while history and typeset input continue to show exactly what the user typed. REPL commands such as `:plot` continue to use Enter so command syntax is never rewritten.

The editor REPL accepts `:help` and `:precision N`. The help response lists supported GUI commands; precision uses the interpreter's decimal limit. In the source editor, Ctrl+X cuts the current line when no text is selected.

### Status Bar

The 26px accent bar anchors the workspace. It shows diagnostic counts and, when width permits, the current operation. The right side shows line and column, indentation, encoding, line ending, and SCAS file type in the document workspace. REPL and Graphing supply their own context. Text color is selected automatically for contrast against each theme's accent.

### Save and unsaved changes

File actions and shortcuts include Save, Save As, New File, and Quick Open. A first save opens Save As, which requires a `.scas` path and rejects overwriting another existing file. New, opening another file, and window close pause on dirty content with Save, Discard, and Cancel choices.

## Do's and Don'ts

### Do:

- **Do** preserve the title bar → activity rail/Explorer → editor and breadcrumbs → optional preview/output → status bar topology.
- **Do** keep the editor dominant on startup and make preview and output easy to summon.
- **Do** consume the shared TOML theme roles and preserve all twelve bundled themes plus external loading.
- **Do** use one-pixel separators, 2px control corners, compact spacing, and the shared 160ms state motion.
- **Do** pair color with text, shape, count, or position for dirty, live, run, selection, and diagnostic state.

### Don't:

- **Don't** turn persistent panels into rounded cards, add gradients, or introduce ambient workspace shadows.
- **Don't** spend accent color on decoration or allow syntax colors to leak into general chrome.
- **Don't** replace the custom vector family with emoji, icon fonts, or mixed stroke weights.
- **Don't** force preview or output open at startup; show output when a run needs it.
- **Don't** enlarge routine chrome beyond the established 11–13px range or loosen the compact density without a platform accessibility reason.
