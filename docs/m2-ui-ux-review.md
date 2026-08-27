# M2 UI/UX Review Gate

This review is the release gate for the M2 scenes and composition work. It checks
whether Rivulet is usable as a desktop recording and streaming application, not
only whether individual features exist. It applies the common and M2-specific
criteria from [`milestone-quality-gates.md`](milestone-quality-gates.md). The
review must be completed before M2 is marked done or M3 work is treated as the
primary product focus.

## Exit criteria

The review passes when all of the following are true:

- Every core workflow in the workflow matrix has a recorded result on Windows,
  Linux, and macOS, or an explicit `N/A` with a documented platform reason.
- No finding is open with severity `Blocker` or `Critical`.
- All `High` findings have either been fixed and retested or accepted by the
  release owner with a linked issue and target milestone.
- Keyboard, focus, contrast, scaling, and error-state checks pass in both dark
  and light themes where the platform supports the theme.
- The evidence bundle contains screenshots or recordings for every failed,
  deferred, or platform-specific check.
- The completed report is committed as `docs/m2-ui-ux-review-report.md` (copy
  the template below) and linked from the M2 release notes.
- The report explicitly states whether evidence is **manual**, **automated**, or
  **blocked**. A successful build is evidence of compilation only, not of usable
  capture, permissions, rendering, audio, or display behavior.

This is a product-quality gate. It does not replace automated unit, integration,
or CI tests.

## Review setup

Record the following before testing:

| Field | Value |
| --- | --- |
| Commit / build | `____________________________` |
| Reviewer(s) | `____________________________` |
| Date (UTC) | `____________________________` |
| Windows version / GPU | `____________________________` |
| Linux desktop / display server | `____________________________` |
| macOS version / GPU | `____________________________` |
| Display scale(s) tested | `100% / 125% / 150% / other: ______` |
| Theme(s) tested | `Dark / Light / System` |

Use a clean profile and a representative test fixture set: at least two scenes,
one nested/organized scene if available, one monitor, one window, one game window,
one camera or image source, and one audio input. Do not use real stream keys in
screenshots or recordings.

## Workflow matrix

Mark each row `PASS`, `FAIL`, `BLOCKED`, or `N/A`, and include an evidence reference.
`PASS` requires an actual manual or reproducible integration observation on the
listed platform; unit tests and compilation alone cannot turn a platform row into
`PASS`. Use `BLOCKED` when the required hardware/display/session is unavailable.
Use `N/A` only when the workflow is intentionally unsupported and link the parity
issue or roadmap decision.

| ID | Workflow | Expected result | Windows | Linux | macOS | Evidence / issue |
| --- | --- | --- | --- | --- | --- | --- |
| W01 | Launch and recover | App opens without a blank/unresponsive state; startup and error status are understandable |  |  |  |  |
| W02 | Select monitor, window, and game source | Source selection remains independent from window selection; selected target is visible in the preview |  |  |  |  |
| W03 | Start and stop recording | Start/stop state, timer, FPS, output path, and failures are clear; a playable file is produced |  |  |  |  |
| W04 | No-frame failure | A missing or stalled capture source produces a visible error and does not leave a false running state |  |  |  |  |
| W05 | Create, rename, switch, and remove scenes | Active scene is obvious; destructive actions are understandable and recoverable |  |  |  |  |
| W06 | Collections and profiles | Current collection/profile is visible and switching does not silently alter another context |  |  |  |  |
| W07 | Duplicate scene and source | A duplicate has a distinct identity and does not acquire unintended scene bindings |  |  |  |  |
| W08 | Edit composition | Transform, crop, visibility, lock, and z-order changes affect the selected scene binding only |  |  |  |  |
| W09 | Undo and redo | Ctrl+Z/Ctrl+Y and visible controls are discoverable, deterministic, and do not affect text fields |  |  |  |  |
| W10 | Scene transitions | Cut and Fade are understandable, non-blocking, and show progress without freezing the UI |  |  |  |  |
| W11 | Live preview refresh | Preview and window list refresh correctly; manual refresh does not unexpectedly change the source |  |  |  |  |
| W12 | Audio and recording status | Mixer levels, mute/monitoring state, skipped filters, and errors are distinguishable |  |  |  |  |
| W13 | Settings and persistence | Theme, language, recording settings, and relevant preferences survive restart as documented |  |  |  |  |
| W14 | Recovery and diagnostics | A failure can be located in the GUI, daily log, or pre-Rust diagnostic path without exposing secrets |  |  |  |  |
| W15 | Update path | Update status, failure, cancellation, and restart behavior are clear and recoverable |  |  |  |  |

## Heuristic and accessibility checks

Run these checks for Record, Scenes, and Settings at minimum:

| ID | Check | Pass condition | Result / evidence |
| --- | --- | --- | --- |
| H01 | Information hierarchy | Page title, active navigation item, primary action, and current status are clear within three seconds |  |
| H02 | Action consistency | Similar actions use the same icon, label, placement, and disabled-state behavior |  |
| H03 | Source identity | Source, window, monitor, and scene names cannot be confused; selected values remain visible |  |
| H04 | Feedback | Every start, stop, save, duplicate, delete, refresh, and failed action gives immediate feedback |  |
| H05 | Error recovery | Errors explain what happened and the next safe action; no console-only user error remains |  |
| H06 | Keyboard access | Primary workflows and scene history are usable without a mouse; focus order is logical |  |
| H07 | Focus visibility | Focus is visible in both themes and is not conveyed by color alone |  |
| H08 | Contrast | Text, controls, status colors, and disabled states remain readable in dark and light themes |  |
| H09 | Text scaling | At 125% and 150% display scaling no important text, button, or status is clipped |  |
| H10 | Small window | At the minimum supported window size there is no overlapping or unreachable primary action |  |
| H11 | Long content | Long scene/source/window names wrap or truncate predictably without changing layout geometry |  |
| H12 | Responsiveness | No workflow blocks the UI thread; transitions, refreshes, and capture failures leave the app responsive |  |
| H13 | Localization | English and German labels fit their containers and do not mix languages in one workflow |  |
| H14 | Privacy | Logs, screenshots, and diagnostics do not contain stream keys or unnecessary personal paths |  |

## Test matrix

At minimum, execute the workflow matrix on these profiles. Expand the matrix when
a release targets additional hardware.

| Profile | OS / display | Required checks |
| --- | --- | --- |
| P1 | Windows, 100%, dark | W01-W15, H01-H14 |
| P2 | Windows, 150%, light | W01-W03, W05-W11, W13, H01-H14 |
| P3 | Linux X11, 100%, dark | W01-W15, H01-H14 |
| P4 | Linux Wayland, 125%, light | W01-W04, W08-W14, H01-H14 |
| P5 | macOS, Retina/default, system | W01-W03, W05-W11, W13-W15, H01-H14 |

If a platform cannot execute a workflow, record the reason and link the platform
parity issue. `N/A` is not a passing result for a feature advertised as
cross-platform.

## Evidence and findings

Use stable filenames such as `m2-uiux-P1-W03.mp4` or
`m2-uiux-P2-H09.png`. Redact usernames, local paths, stream keys, and window
content before committing evidence. Store large recordings outside Git and link
to the review artifact or CI run.

| ID | Severity | Profile / check | Finding | Reproduction | Owner / issue | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F-001 | Blocker/Critical/High/Medium/Low |  |  |  |  | Open |

Do not leave the example finding in a completed report. Findings must have a stable
ID, severity, owner, follow-up milestone/issue when deferred, and a retest result.

Severity guidance:

- `Blocker`: recording, recovery, or navigation cannot be completed.
- `Critical`: data loss, unsafe output, an unrecoverable freeze, or a release
  platform cannot perform its advertised core workflow.
- `High`: a core workflow is misleading, inaccessible, or repeatedly fails but
  has a workaround.
- `Medium`: substantial friction or inconsistent feedback without data loss.
- `Low`: polish, copy, spacing, or non-blocking visual issue.

## Reproducible review procedure

1. Build the exact commit under review with the normal platform instructions.
2. Run automated checks first:

   ```bash
   cargo fmt --all -- --check
   cargo test --workspace
   cargo clippy --workspace --all-targets -- -D warnings
   ```

3. Create a clean test profile and launch Rivulet with diagnostics enabled. Keep
   the daily log and launcher diagnostics for the evidence bundle.
4. Execute the workflows in order on each available platform profile, then repeat
   the accessibility checks at the display scales and themes listed in the matrix.
5. Record every result immediately. Do not mark a row `PASS` from memory. For
   platform-dependent behavior, attach a screenshot, recording, or test-run link.
6. For unavailable hardware, display servers, permissions, or platform features,
   record `BLOCKED`/`N/A`, the reason, and the linked parity issue; do not infer
   success from another platform's result.
7. Re-test every fixed finding on the same profile and record the new evidence.
8. Complete the report, obtain a second reviewer for all `High` or higher
   findings, and link the report from the release notes.

## Report template

Copy this section to `docs/m2-ui-ux-review-report.md` for each review. Replace all
blank cells and remove placeholder rows before committing the report:

```markdown
# M2 UI/UX Review Report

- Commit/build:
- Review date (UTC):
- Reviewers:
- Profiles executed:
- Overall result: PASS / CONDITIONAL / FAIL

## Summary

- Blocker/Critical findings:
- High findings:
- Deferred Medium/Low findings:
- Known platform limitations:

## Results

Link the completed workflow and heuristic tables from `m2-ui-ux-review.md`,
or paste the results here.

## Findings

| ID | Severity | Description | Issue / owner | Retest |
| --- | --- | --- | --- | --- |

## Decision

- [ ] M2 UI/UX gate passed; M2 may be marked complete.
- [ ] Conditional pass; listed follow-ups are assigned to a milestone.
- [ ] Failed; release-blocking work remains.
```

The report is the durable decision record. Keep this checklist stable and add
new checks only when a regression or new workflow justifies them.
