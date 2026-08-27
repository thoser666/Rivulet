# OBS upstream feature check

`check-obs-upstream.py` checks the latest OBS Studio GitHub release and extracts
release-note lines that look like feature additions. Candidates are compared
with the versioned catalog in `scripts/obs-features.json` and printed as either
already represented or needing review.

## Automation

`.github/workflows/obs-upstream.yml` runs weekly and can also be started with
`workflow_dispatch`. It publishes the report in the Actions step summary and
uploads it as an artifact. The workflow does not modify `develop`, create issues,
or automatically change the parity checklist: release notes are editorial and
need human classification.

## Local usage

```bash
python3 scripts/check-obs-upstream.py --self-test
python3 scripts/check-obs-upstream.py
python3 scripts/check-obs-upstream.py --fixture scripts/obs-upstream-fixture.json
```

A GitHub API/network failure exits as **unverified** (code 2); it must not be
interpreted as proof of feature parity. After reviewing a candidate, update
`obs-features.json`, the README checklist, and the appropriate milestone/issue.
