# M10 – Extensible UI & Plugin Platform

Rivulet reserves a separate milestone for customizable layouts and UI plugins.
This work must not weaken streaming reliability or expose credentials.

## Work packages

- **P1 – Persistable UI layout:** versioned workspace state, migrations, safe defaults,
  and no secrets or runtime handles in storage.
- **P2 – View registry:** stable IDs for built-in and optional views; navigation, Help,
  and accessibility derive from one registry.
- **P3 – Declarative UI plugins:** manifest-defined panels, menu items, help pages,
  enable/disable state, and API compatibility checks.
- **P4 – Permission model:** explicit capabilities for UI, network, filesystem,
  capture, audio, and secrets; sensitive capabilities denied by default.
- **P5 – Isolated execution:** preferably WASM, with timeouts, resource limits,
  and failure isolation so a plugin cannot terminate the GUI.
- **P6 – Quality gate:** compatibility, migration, accessibility, UX, performance,
  security, and example-plugin checks.

## M10 completion gate

M10 is complete only after the common checks in
[`milestone-quality-gates.md`](milestone-quality-gates.md) and all M10-specific
checks pass. The final report must include:

- layout persistence round-trip and migrations from every supported schema;
- registry ordering, duplicate-ID handling, localization, keyboard focus, and labels;
- malformed/incompatible manifest rejection and explicit capability prompts;
- proof that secrets and runtime handles are excluded from persisted state;
- timeout, cancellation, crash-isolation, and disabled-plugin recovery tests;
- CPU, memory, startup, frame-time, and idle-repaint measurements;
- Windows, Linux, and macOS results, or a documented `BLOCKED`/`N/A` reason.

No Blocker or Critical finding may remain. High findings must be fixed or assigned
to a named follow-up issue and milestone. The result is recorded in
`docs/m10-quality-report.md` as `PASS`, `CONDITIONAL`, or `FAIL`.

## Guardrails

- `eframe::Storage` may contain layout and non-sensitive preferences only.
- Stream keys, tokens, passwords, and private endpoints remain in the OS credential
  store and are never exposed to a UI plugin by default.
- Native dynamic libraries are not the initial plugin format because they share the
  GUI process and can crash or compromise it.
- Each plugin must declare an API version and requested capabilities.

The first implementation target is P1. P2 follows once the persistence schema and
migration behavior are stable.
