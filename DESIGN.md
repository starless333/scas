---
name: SCAS Desktop IDE
description: A restrained native workspace for writing, running, diagnosing, and typesetting mathematical programs in one view.
colors:
  workspace: "#181818"
  panel: "#1f1f1f"
  editor: "#1e1e1e"
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
    fontFamily: "egui Proportional, system-ui, sans-serif"
    fontSize: "16px"
    fontWeight: 600
  body:
    fontFamily: "egui Proportional, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
  control:
    fontFamily: "egui Proportional, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 400
  label:
    fontFamily: "egui Proportional, system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 600
  code:
    fontFamily: "egui Monospace, ui-monospace, monospace"
    fontSize: "15px"
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
    backgroundColor: "{colors.panel}"
    textColor: "{colors.text}"
    typography: "{typography.control}"
    rounded: "{rounded.none}"
    padding: "5px 8px"
    height: "36px"
  activity-button:
    backgroundColor: "{colors.workspace}"
    textColor: "{colors.muted}"
    rounded: "{rounded.control}"
    size: "40px 38px"
  editor-tab-active:
    backgroundColor: "{colors.panel}"
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
    height: "22px"
---

# Design System: SCAS Desktop IDE

## Overview

**Creative North Star: "The Mathematical Workbench"**

SCAS is a dense, native Operate-mode desktop IDE: familiar enough to trust immediately, quiet enough to keep source and rendered mathematics in focus. Its workspace topology follows established developer tools, while its restraint, sparse accent usage, and precise spacing take their cue from Zed. The visual system is functional, flat, and theme-led rather than branded through ornament.

The signature workflow is simultaneous source and live typeset output. Explorer, editor, preview, diagnostics, program output, and status remain visible as one connected instrument; the live mathematical preview is the product-specific focal surface, never a detached secondary destination. Twelve bundled developer themes share the same semantic TOML contract, with Dark Modern as the default, and external theme loading remains a first-class extension point.

**Key Characteristics:**

- Dense, flat panels divided by quiet one-pixel structure.
- Compact 11–13px interface chrome around a 15px code surface.
- A stable IDE topology with collapsible and resizable supporting panes.
- One theme accent reserved for selection, active state, live state, and status.
- Custom vector icons drawn in a consistent 1.6px stroke language.
- Source and rendered mathematics held in direct visual conversation.

## Colors

The default Dark Modern palette uses near-neutral charcoal layers, soft gray text, and one familiar editor-blue accent; every bundled or external theme supplies the same seven UI roles and ten syntax roles declared in the frontmatter.

### Primary

- **Workbench Blue:** The sole interactive accent. It marks the Run action, active tabs, live preview state, focus, current-line tint, and the status bar.

### Tertiary

- **Syntax Spectrum:** Plain text, keywords, numbers, strings, comments, documentation, identifiers, operators, delimiters, and render markers receive distinct semantic roles. Themes may change hues, but never collapse the role schema.

### Neutral

- **Workspace Charcoal:** The deepest shell layer, used behind the activity rail and the outer workspace.
- **Panel Charcoal:** The command bar, Explorer, preview, and editor-tab surface.
- **Editor Charcoal:** The source editor, output dock, and rendered-document inset.
- **Soft Foreground:** Primary labels and interface content; it avoids stark white glare.
- **Muted Steel:** Secondary labels, inactive icons, tree glyphs, and separators at reduced opacity.
- **Accent Contrast:** Derived white or near-black text that remains legible on the selected theme's accent.
- **Selection Blue:** Selected and hovered control fill without introducing a second accent.

### Named Rules

**The One Accent Rule.** Accent color communicates action or state; it is not ambient decoration.

**The Semantic Theme Rule.** Components consume named UI and syntax roles, never theme-specific color literals.

## Typography

**Display Font:** egui Proportional with the platform system fallback  
**Body Font:** egui Proportional with the platform system fallback  
**Label/Mono Font:** egui Monospace with the platform monospace fallback

**Character:** Typography is deliberately utilitarian and compact. Weight, case, and semantic color create hierarchy within a narrow 11–16px range so the workspace stays information-dense without becoming noisy.

### Hierarchy

- **Heading** (semibold, 16px): Dialog and rare high-level headings.
- **Body** (regular, 13px): Standard interface copy, tree entries, output, and supporting content.
- **Control** (regular, 12px): Menus, buttons, active file tabs, and compact actions.
- **Label** (semibold, 11px): Uppercase panel titles, OUTPUT/PROBLEMS tabs, state, metadata, and the status bar.
- **Code** (regular, 15px default): Editable SCAS source. Users may scale or replace this font without changing the chrome hierarchy.

### Named Rules

**The Chrome Ceiling Rule.** Routine interface chrome stays between 11px and 13px; 15–16px is reserved for code, rendered-document headings, and exceptional hierarchy.

## Layout

The workspace uses a fixed 36px menu/title bar, a fixed 48px activity rail, an open-by-default Explorer at 240px, a central source editor, a right-side live preview occupying roughly 46% of the post-rail width, a 170px bottom output dock, and a 22px accent status bar. The Explorer resizes from 180–480px, the preview from 280–900px, and the output dock from 80–420px; each can be collapsed or resized without changing the underlying topology.

Spacing follows a compact 2/4/6/8/10/12px rhythm. Panel headers use 8–10px horizontal insets and 5–7px vertical insets. Explorer hierarchy advances by 12px per depth level, with explicit disclosure, folder, and file cues. The rendered-document inset uses a more generous 18px to distinguish the typeset artifact from surrounding chrome.

The active editor tab remains flat within its panel and uses a 2px accent underline. Dirty state is redundant: the filename carries a dot and the adjacent “Modified” label states the condition. Dock tabs keep OUTPUT and PROBLEMS simultaneously discoverable, including a live problem count.

**The Stable Topology Rule.** Resizing and collapsing redistribute work area but do not reorder navigation, source, preview, output, or status.

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

- **Command bar:** A 36px integrated title/menu strip with product mark, familiar File/View/Run menus, document title, Settings, and a right-aligned Run action.
- **Activity rail:** A fixed 48px rail containing 40×38px buttons. Selected state uses selection fill; active and hovered icons move from muted to primary text color.
- **Explorer:** Open by default, compact, resizable, and hierarchical. Disclosure triangles plus custom folder/file outlines make object type and collapsed state redundant.

### Editor Tab

The active tab is flat and inseparable from the editor surface. A 2px accent underline marks activity; filename, dirty dot, “Modified” text, language label, actions, and close state stay legible in one compact row.

### Live Typeset Preview

The preview is the signature component: a resizable side panel with DOCUMENT PREVIEW, LIVE/UPDATING state, rendering-mode toggle, Copy TeX action, and an inset rendered artifact. It updates beside source, preserves the last good preview during editing errors, and keeps LaTeX source available as a subordinate disclosure.

### Output Dock

OUTPUT and PROBLEMS share a resizable bottom dock. The selected tab uses the selection treatment, PROBLEMS always exposes its count, output stays selectable and copyable, and diagnostics retain line and column context.

### Status Bar

The 22px accent bar anchors the workspace and shows READY/WORKING, the current operation, encoding, cursor line, and problem count. Text color is selected automatically for contrast against each theme's accent.

## Do's and Don'ts

### Do:

- **Do** preserve the menu bar → activity rail/Explorer → editor/preview → output dock → status bar topology.
- **Do** keep source and the live mathematical result visible together whenever width permits.
- **Do** consume the shared TOML theme roles and preserve all twelve bundled themes plus external loading.
- **Do** use one-pixel separators, 2px control corners, compact spacing, and the shared 160ms state motion.
- **Do** pair color with text, shape, count, or position for dirty, live, run, selection, and diagnostic state.

### Don't:

- **Don't** turn persistent panels into rounded cards, add gradients, or introduce ambient workspace shadows.
- **Don't** spend accent color on decoration or allow syntax colors to leak into general chrome.
- **Don't** replace the custom vector family with emoji, icon fonts, or mixed stroke weights.
- **Don't** hide live typeset output behind a separate workflow when a simultaneous preview can fit.
- **Don't** enlarge routine chrome beyond the established 11–13px range or loosen the compact density without a platform accessibility reason.
