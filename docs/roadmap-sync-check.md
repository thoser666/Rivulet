# Roadmap sync check

The milestone sequence is **owned by GitHub**: each milestone's title is the
canonical `M<n> – Title` name (e.g. `M6 – Creator Toolkit & Interactivity`),
and its issue tracker state is the source of truth for what is planned,
open, or done. Two repository documents mirror that sequence and must never
drift from it:

- **README.md → 🚀 Roadmap → Milestone overview** — one table row per
  milestone, in ascending order, each carrying a shields badge whose GitHub
  milestone id must point at the milestone with exactly that title.
- **docs/milestone-quality-gates.md** — the cross-cutting
  resource-efficiency table (`| M<n> | … |`) lists the same milestones in the
  same order, and every milestone from M2 onward has a `### M<n>:` gate
  section (M0/M1 are the recording foundation and predate the gates document).

A milestone rename or renumber (such as the M6–M11 shift) therefore touches
three places; `scripts/check-roadmap-sync.py` makes a forgotten one fail CI.

## What the checker validates

Local document checks (always run, no network):

1. The README overview rows form a contiguous ascending run of `M<n>`
   numbers with no duplicates, and every row carries a GitHub milestone
   badge.
2. The gates-doc resource table lists exactly the same milestone numbers in
   the same order as the README table.
3. Every README milestone `M<n>` with `n >= M2` has a `### M<n>:` gate
   section, and no gate section exists for a milestone the README does not
   list.

GitHub comparison (live REST call in CI, or `--fixture` offline):

4. Each README row label `M<n> – Title` matches the title of exactly one
   GitHub milestone.
5. The badge id in the README row equals that GitHub milestone's number, so
   a badge cannot silently point at the wrong milestone after a renumber.
6. The `M<n>` prefixes extracted from the GitHub titles form the same
   contiguous set as the README rows.

## Exit codes and modes

- `0` — everything is in sync (local + GitHub).
- `1` — a drift was found (CI fails; fix the README, the gates doc, or the
  GitHub milestone).
- `2` — the local docs are consistent, but the GitHub milestone list could
  not be fetched (API unreachable), so the live half is unverified. CI treats
  this as a failure so an outage cannot silently disable the check.

Run it like the other scripted checks:

```bash
python3 scripts/check-roadmap-sync.py --self-test   # deterministic unit tests
python3 scripts/check-roadmap-sync.py               # local + live GitHub check
python3 scripts/check-roadmap-sync.py --fixture ms.json   # offline with a
                                            # JSON array of {number, title}
```

## Wiring

CI runs the checker as the dedicated **Roadmap-Sync Check** job (pure
Python, no Rust build) with `issues: read` so the milestone REST call works
under the workflow token, and the CI aggregate requires it. The
`ci_pinning` suite keeps the wiring and the canonical full milestone names
(M4/M8/M11 in the overview table) pinned so neither can drift unnoticed.
