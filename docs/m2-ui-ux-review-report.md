# M2 UI/UX Review Report

## Functional verification update (2026-08-27)

The functional M2 workflows are implemented: scene collections/profiles with JSON
import/export, scene/source duplication, per-scene hotkeys, title-based opt-in
auto-switching, browser source configuration, deterministic snapshots, Studio Mode,
and the in-window multi-view/projector fallback controls. The remaining release
requirement is the manual cross-platform review documented below. Native fullscreen
projector routing and native source-pixel rendering are explicitly tracked as
post-M2 renderer work.


> Template status: not completed. Copy this file or fill it for the commit under review.
> Do not mark M2 complete until the decision below is recorded.

- Commit/build:
- Review date (UTC):
- Reviewers:
- Profiles executed:
- Overall result: `PASS` / `CONDITIONAL` / `FAIL`

## Summary

- Blocker/Critical findings:
- High findings:
- Deferred Medium/Low findings:
- Known platform limitations:

## Workflow results

| Profile | Passed | Failed | Blocked / N/A | Evidence |
| --- | --- | --- | --- | --- |
| P1 Windows 100% dark |  |  |  |  |
| P2 Windows 150% light |  |  |  |  |
| P3 Linux X11 100% dark |  |  |  |  |
| P4 Linux Wayland 125% light |  |  |  |  |
| P5 macOS Retina/system |  |  |  |  |

See [`m2-ui-ux-review.md`](m2-ui-ux-review.md) for the complete workflow and
heuristic matrices. Record every failed, blocked, or deferred check below.

## Findings

| ID | Severity | Profile / check | Description | Reproduction | Issue / owner | Retest |
| --- | --- | --- | --- | --- | --- | --- |

## Decision

- [ ] M2 UI/UX gate passed; M2 may be marked complete.
- [ ] Conditional pass; every follow-up is assigned to a milestone and owner.
- [ ] Failed; release-blocking work remains.

Release notes / tracking issue:
