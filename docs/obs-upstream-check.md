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
uploads it as an artifact. It also generates the committed-format document
`docs/obs-vision-candidates.md` in the runner and includes it in the artifact.
The workflow does not modify `develop`, create issues, or automatically change
the parity checklist: release notes are editorial and need human classification.
The generated candidate document is a review queue, not product approval.

## Local usage

```bash
python3 scripts/check-obs-upstream.py --self-test
python3 scripts/check-obs-upstream.py
python3 scripts/check-obs-upstream.py --fixture scripts/obs-upstream-fixture.json
```

A GitHub API/network failure exits as **unverified** (code 2); it must not be
interpreted as proof of feature parity. After reviewing a candidate, update
`obs-features.json`, the README checklist, and the appropriate milestone/issue
only when the candidate fits the product vision. Strong-fit candidates can be
copied into `docs/obs-vision-candidates.md` with `--update-doc`; after review,
maintainers may promote them into [`obs-vision-roadmap.md`](obs-vision-roadmap.md), the README roadmap, and a milestone issue.
