# Notebook contracts (schema 1)

This document freezes the phase 0 contracts implemented by
`scas::notebook`. A notebook is UTF-8 `.scas` source. JSON, cached output, and
the reproducibility manifest are derived artifacts and never canonical input.

## Cells and identifiers

`// %% optional label` is reserved as an editor boundary and remains an
ordinary line comment to every SCAS execution path. The parser also creates
conservative boundaries between top-level statements and keeps consecutive
`///` lines in one prose cell. Cells cover the source exactly, without gaps or
normalization, using half-open UTF-8 byte spans plus one-based line/column
positions.

Cell IDs are document-revision scoped: `cell-1`, `cell-2`, and so on in source
order. They remain stable when text changes do not add, remove, or reorder an
earlier cell. Cross-revision persistent IDs are intentionally deferred to an
optional sidecar; generated IDs are never inserted into `.scas` source.

## Execution and state

- Run cell uses the current persistent session and is not a reproducible run.
- Run above and Run all create a new symbol table, interpreter, output buffer,
  assumptions, precision state, I/O state reference, and step counters.
- Only a successful Run all is `clean` and receives a manifest.
- Editing cell N marks N and all later previously-run cells stale. This may
  over-report dependencies, but never under-reports them.
- Ordered execution stops at the first failed, cancelled, or input-blocked
  cell and marks remaining required cells blocked.
- States are `not_run`, `queued`, `running`, `current`, `stale`, `failed`,
  `blocked`, and `cancelled`; clients must show their text or shape, not color
  alone.
- Every command increments a session generation. Clients accept a result only
  when its generation is still current. Events have a zero-based sequence
  within that generation.
- Native interruption is cooperative and checked at each evaluator step.
  Browser interruption terminates and recreates the Web Worker, because a
  worker running synchronous Wasm cannot receive an interrupt message.
- An exhausted resource limit is a diagnostic event. Missing interactive
  input is an input event and blocked state, never a UI-thread prompt inside
  the shared kernel.

## Output events

The schema-1 kinds are `text`, `stdout`, `value`, `math`, `verification`,
`diagnostic`, and `input`. Each event contains:

- document-scoped cell ID and exact source span;
- session generation and ordered sequence number;
- kind;
- mandatory plain-text fallback;
- kind-specific structured `data` (for example LaTeX, verification status,
  diagnostic severity, or an input prompt).

The CLI emits these objects as JSON Lines. Native and browser clients consume
the same Rust event producer and must not rerun mathematics to enrich output.

## Trust and host capabilities

Opening or parsing never executes source. Sessions record `local`, `untrusted`,
or `trusted` provenance. An explicit Run action is required in every state;
trust is not an auto-run switch. Before native execution, clients should show
the capability scan: imports, input, file reads, and file writes. Browser
sessions use only virtual files and queued input; downloads remain an explicit
host action. Rich payloads contain no arbitrary HTML or JavaScript.

## Reproducibility manifest

A successful Run all emits schema version, SCAS version and target, SHA-256 of
the exact source bytes, SHA-256 for readable imported modules, decimal
precision, every deterministic resource limit, optional random seed and input
data hashes, and a verification summary. A timestamp is deliberately omitted
because it is metadata rather than computational input. Missing imports make
execution fail, so a successful manifest never silently approves an unread
import.

The fixture `examples/notebook-phase1.scas` is run by library, CLI, and browser
protocol tests. Equivalent output means the same ordered event kinds and text,
cell states, source hash, and verification summary; target metadata may differ.
