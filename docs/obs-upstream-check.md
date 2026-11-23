# OBS upstream feature and vision check

`check-obs-upstream.py` checks the latest OBS Studio GitHub release and extracts
release-note lines that look like feature additions. Candidates are compared
with the versioned catalog in `scripts/obs-features.json` and classified against
Rivulet's product vision in `scripts/vision-criteria.json`.

## Vision decisions

The report assigns each new candidate one of three advisory decisions:

- **strong-fit**: matches at least two vision pillars; prioritize roadmap
  evaluation because it supports Rivulet's differentiation.
- **review**: matches one pillar; assess scope, user value, and opportunity cost
  before adding it.
- **not-aligned**: matches no pillar; do not add it solely to chase OBS parity.

The current pillars are deterministic/automatable workflows, an embeddable Rust
engine, modern efficient rendering, cross-platform parity, privacy and user
control, and direct streamer value. Keyword matching is deliberately a first
triage only; maintainers make the final product decision.

## Automation

`.github/workflows/obs-upstream.yml` runs weekly and can also be started with
`workflow_dispatch`. It publishes the report in the Actions step summary and
uploads it as an artifact. It also generates the committed-format documents
`docs/obs-vision-candidates.md` and `docs/community-wish-candidates.md` in the
runner and includes them in the artifact.

### Weekly review issue

Instead of only living in the run artifact, the merged report is published as
a GitHub issue labelled `weekly-vision-review` (title `Weekly vision review —
<date> — OBS <tag>`). The issue body is the full report plus a **review
checklist**: every OBS release-note candidate and every community wish that
still needs a decision appears as an unchecked `- [ ]` box, so reviewers can
tick items as they triage them. A rerun of the same week updates that issue;
a new week closes older open review issues, keeping the label a rolling single
review queue. Creating/updating the issue needs `issues: write`; the workflow
still never modifies `develop` or the parity checklist — release notes and
wishes are editorial and need human classification. The generated candidate
documents are a review queue, not product approval.

### One issue per candidate (deduplicated)

Every release-note candidate that still needs a decision also gets its own
issue, so a single upstream change can be discussed on its own thread. The
sweep is idempotent: it matches the candidate against the tracked issues —
open **and** closed — by *upstream title*, ignoring the `OBS: ` prefix, an
upstream PR reference (the legacy `[author] (#123)` suffix or the markdown link
`([#123](…))`), a trailing `[author]` tag and the `— <decision> (<pillars>)`
suffix, and updates
the existing thread's body (current release tag, vision decision, quoted
release note) instead of creating another one. Only titles that no tracked
issue carries are created, and a second sweep over the same release notes
reports them all as `updated`. Comments and labels on an existing thread
therefore stay attached to the same upstream change.

Closed threads are part of the key on purpose: once maintainers closed a
candidate, reopening it would undo that decision and refiling it would put the
same upstream change back into the queue every week. A closed match is
reported as `left closed` and left untouched — no reopen, no body rewrite, no
new issue — while the report and the review checklist still list the candidate
so the decision stays visible. An open thread wins over a closed one when both
exist. The tracked fetch reads `state=all` (newest activity first, at most
three pages), while the community-wish sweep below still reads open issues
only.

New issues are reserved for candidates that carry a product decision:
`strong-fit` and `review`. A `not-aligned` note never grows the tracker; it
stays in the weekly report (and its checklist row) for the maintainer who
asked why OBS parity does not justify the change. Already-represented
candidates are skipped the same way. Each new issue carries the `feature` and
`obs-upstream` labels, so a sweep thread is distinguishable from the
`weekly-vision-review` issue and from a plain community wish. The workflow
creates `obs-upstream` itself (idempotently, via `gh label create --force`) and
fails if that does not work: GitHub silently drops unknown labels at issue
creation, so a missing label would otherwise leave new threads unlabelled
without any error.

### Rivulet open-issue sweep (community wishes)

The same run also fetches this repository's open issues and classifies feature
wishes against the same `scripts/vision-criteria.json` — one common review
report with two sections (OBS release notes and the issue sweep). Bug reports
and PRs are excluded; `feature`/`idea`/`wish`-labelled issues and unlabelled
titles that read like additions are triaged. `enhancement`/`epic` marks
roadmap-tracked milestone work: those issues are listed in a dedicated
"Roadmap-tracked" section and excluded from the wish review, so the weekly
report shows real community wishes instead of the repo's own milestone backlog.
Wishes whose wording matches a roadmap catalog entry land in an "Already
represented" list instead of the review table, and strong-fit wishes are copied
into `docs/community-wish-candidates.md` (kept in the artifact next to the OBS
candidates). The same advisory policy applies: keyword triage first, maintainer
review before a wish reaches the roadmap.

### Delta tracking across runs

The report shows `Previous checked release:` — the tag the checker verified on
the run before. That tag is recorded in `scripts/.obs-upstream-state.json`, a
gitignored runtime artifact the workflow restores from and saves back to the
**actions cache** (`obs-upstream-state-*`, same pattern as the fuzz corpus). No
repo *content* write is involved, so the workflow keeps `contents: read` plus
`issues: write` (the open-issue sweep and the weekly review issue) — never
write access to the codebase. On the first run (no cache entry yet) the
previous tag reads as `none`; a corrupt state file degrades to `none` as well
instead of failing the run.

### Loud failures

The workflow fails loudly instead of masking problems: `continue-on-error`
is not used, so a checker error (for example the "candidate markers missing"
case that used to silently empty the report) fails the run with its
traceback. After the check, an explicit guard asserts the report file is
non-empty and carries both section headers (`## OBS upstream check` and
`## Rivulet open issues`); an empty or malformed report aborts the run with a
clear `::error::` annotation. The report is still published to the step
summary and uploaded as an artifact on failure (`if: always()`) so a broken
run stays diagnosable.

## Local usage

```bash
python3 scripts/check-obs-upstream.py --self-test
python3 scripts/check-obs-upstream.py
python3 scripts/check-obs-upstream.py --fixture scripts/obs-upstream-fixture.json
# Offline sweep of Rivulet issues (no network):
python3 scripts/check-obs-upstream.py --fixture scripts/obs-upstream-fixture.json \
  --issues-file issues.json
# Skip the issue sweep entirely:
python3 scripts/check-obs-upstream.py --no-issues
```

A GitHub API/network failure exits as **unverified** (code 2); it must not be
interpreted as proof of feature parity. After reviewing a candidate, update
`obs-features.json`, the README checklist, and the appropriate milestone/issue
only when the candidate fits the product vision. Strong-fit candidates can be
copied into `docs/obs-vision-candidates.md` with `--update-doc`; strong-fit
community wishes land in `docs/community-wish-candidates.md` the same way.
After review, maintainers may promote them into
[`obs-vision-roadmap.md`](obs-vision-roadmap.md), the README roadmap, and a
milestone issue.
