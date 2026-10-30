# Deprecation and migration policy

How a public name in `rivulet-core` is retired, how long a consumer has to
migrate, and where the migration path is written down. This is the written
answer the M8 gate asked for (finding F-M8-005, issue
[#344](https://github.com/thoser666/Rivulet/issues/344)); the contract itself is
[`m8-api-contract.md`](m8-api-contract.md), the generated surface is
[`api-overview.md`](api-overview.md).

The policy is not prose alone: the *Guards* section at the end names the tests
and scripts that fail when this document and the code disagree.

## Version scheme, honestly

Rivulet is pre-1.0. Cargo treats a `0.x` minor bump as breaking, so `0.66.0`
may break `0.65.0` consumers — that is the semver rule, and pretending otherwise
would be worse than naming what is actually guaranteed:

- **No public name is removed inside a minor line.** `0.65.0-alpha.57` does not
  remove something that `0.65.0-alpha.56` still offered.
- **Every removal is announced for at least one minor release.** A name
  deprecated in `0.65` is removable in `0.66` at the earliest, never in the same
  minor line, and never in an alpha/patch increment of it.
- **Deprecation is a warning, not a removal.** The item stays callable, stays
  re-exported from the crate root, and keeps its documented behaviour for the
  whole window.

## Deprecating a name

A deprecation is a promise with four parts, all in the same pull request:

1. **The attribute.**
   ```rust
   #[deprecated(
       since = "0.65.0-alpha.55",
       note = "availability depends on the codec; use detect_encoder_availability_for_codec(encoder, codec)"
   )]
   ```
   `since` is the version that ships the deprecation, the `note` names the
   replacement **by path or exact function name** — "deprecated" alone is not a
   migration path.
2. **The changelog.** An entry under `## [Unreleased]` names the item, says what
   it answered wrongly or which need it no longer serves, and names the
   replacement. The first application is the entry for
   `detect_encoder_availability` ([#345](https://github.com/thoser666/Rivulet/issues/345)).
3. **The migration note.** A row in the *Scheduled removals* table below, with
   the earliest removal version. A consumer reading the policy can therefore see
   every deadline in one place.
4. **A test for the replacement.** The replacement answers the question the
   deprecated item was used for; where that cannot be asserted automatically
   (hardware), the reason is written into the contract document. For #345 the
   typed helper and the probe are pinned against each other for every
   (encoder, codec) pair.

Deprecating without a replacement is allowed only when the capability is
withdrawn deliberately; the note then says so explicitly ("no replacement:
…") and the migration row explains what a consumer should do instead.

## Removing or changing a name

A removal — or a change that breaks compilation for an existing caller, which
is the same thing from the consumer's side — requires:

- the deprecation window to have elapsed,
- a row in the *Applied migrations* table below with a non-empty replacement and
  migration note, written **before** the removal merges,
- a `## [Unreleased]` changelog entry that links the deprecation entry,
- `python scripts/generate-api-overview.py` and `--write-baseline`, so the
  generated surface and the removal baseline move together.

Changes that count as removals even though the name survives:

- changing a function signature, including adding a parameter,
- removing or renaming an enum variant, a struct field, a trait method or an
  associated constant,
- adding a field to a public struct that a consumer can construct literally
  (`Scene { .. }`) — new fields belong behind `#[non_exhaustive]`, a constructor,
  or a setter, as `Scene::kind` does with `#[serde(default)]`,
- changing the meaning of a returned value without changing its type; the
  honesty bar from #345 applies: a helper that claims "available" on a machine
  without the hardware is a breaking change in behaviour even if it compiles.

Deprecation exists for exactly these cases. #345 chose **a new function over a
changed signature** (`detect_encoder_availability_for_codec`) precisely so no
existing caller had to be broken to fix the answer.

## What the promise does not cover

- Modules whose own documentation calls them experimental (listed in
  [`api-overview.md`](api-overview.md)) — they may change between alpha
  increments. `PipeWireBackend` and `MetalWgcBackend` are the current examples
  (F-M8-004).
- Test helpers exported for the regression suite.
- Behaviour that is already documented as undefined or platform-dependent.

## Scheduled removals

| Item | Deprecated in | Earliest removal | Replacement |
| --- | --- | --- | --- |
| `rivulet_core::detect_encoder_availability` | 0.65.0-alpha.55 ([#345](https://github.com/thoser666/Rivulet/issues/345)) | 0.66.0 | `detect_encoder_availability_for_codec(encoder, codec)`, or `detect_available_encoders_with_info(codec)` for the full list with `why_not` |

## Applied migrations

Rows are added **before** the removal merges, so a consumer upgrading across a
minor boundary finds every change that can break their build in one table. The
replacement cell must say what to call instead; "removed" is not a migration
note.

| Removed or changed item | Removed in | Replacement | Migration note |
| --- | --- | --- | --- |
| — | — | — | no removal has been applied yet |

## Guards

Three guards keep this document and the code from drifting apart:

- **`api-stability` removal guard** (`rivulet-core/tests/api_stability.rs`):
  `docs/api-overview.md` is the current crate-root surface; the file
  `rivulet-core/tests/api_surface_baseline.txt` is the surface of the last
  released line. Every baseline name missing from the current surface must have
  a row in *Applied migrations* with a replacement and a migration note, or the
  test fails and names the undocumented removals. The baseline is refreshed with
  `python scripts/generate-api-overview.py --write-baseline` **when a release
  line opens**, never to silence a failing guard.
- **Deprecation-note guard** (same suite): every `#[deprecated]` attribute in
  `rivulet-core/src` must carry `since` and a non-empty `note`, so an item cannot
  be retired with a bare "deprecated" and no replacement.
- **Surface generator** (`scripts/generate-api-overview.py --check`, CI Lints
  job): `docs/api-overview.md` must match `rivulet-core/src/lib.rs`, so the table
  the removal guard reads is the code's own surface, not a hand-kept list.
