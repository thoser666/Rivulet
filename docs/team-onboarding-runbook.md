# Team-Onboarding-Runbook — den Review-Gate scharf schalten

> The exact, ordered steps to take **when a second maintainer gets access**.
> Executed top to bottom in one sitting; every step is verifiable. Nothing
> here may be run while the repo is still effectively single-maintainer:
> `required_approving_review_count: 1` blocks every merge (GitHub does not
> allow approving your own pull request), which is why this is a runbook and
> not a settings change.

## Preconditions

- [ ] The second maintainer has accepted a collaborator invite (or is a
      member of the maintainer team) — **access only, nothing else changed**.
- [ ] They can clone, branch and open a PR (verify with a trivial docs PR
      merged through the normal flow).
- [ ] `docs/clean-desk.md` checklist items 1–3 are done for *both* people.
- [ ] CI on `develop` is green.

## Step 1 — Prepare the flip PR (can be done before step 2)

One PR that carries all in-repo changes so docs, guard and tests move
together (the repo's own convention: no drifted states between live config
and its guards):

- `docs/security.md` and `CONTRIBUTING.md`: replace the
  single-maintainer/"automated checks are the review" phrasing with the
  team review requirement.
- `docs/clean-desk.md` + this runbook: mark the transition date.
- `CHANGELOG.md` entry (`chore(security):`).

Wait — the ruleset flip (step 3) happens **after** this PR merges.

## Step 2 — Flip CODEOWNERS default owner

Replace the default owner line in `.github/CODEOWNERS`:

```
*                               @rivulet-maintainers
```

(create the `@rivulet-maintainers` team with both maintainers first, or list
both accounts explicitly if teams are not wanted). Push as a normal PR —
this is itself the first PR that goes through the *old* gate; merge it
right after step 4 or before it, either order works, but docs + owners must
be merged before the ruleset flip lands.

## Step 3 — Ruleset flip (API, in this exact order)

```bash
# 3a. Read the current develop ruleset (grab its id):
gh api repos/thoser666/rivulet/rulesets --jq '.[] | {id, name}'

# 3b. Dump it to a file and patch the pull_request rule:
gh api repos/thoser666/rivulet/rulesets/<ID> > /tmp/rs.json
python3 - <<'EOF'
import json
rs = json.load(open('/tmp/rs.json'))
for rule in rs['rules']:
    if rule['type'] == 'pull_request':
        rule['parameters']['required_approving_review_count'] = 1
# The API rejects read-only fields on update; strip what PATCH doesn't take:
for key in ('id', 'name', 'target', 'source_type', 'source', 'created_at',
            'updated_at', 'current_user_can_bypass'):
    rs.pop(key, None)
json.dump(rs, open('/tmp/rs-patch.json', 'w'))
EOF

# 3c. Apply (PUT replaces the ruleset definition):
gh api -X PUT repos/thoser666/rivulet/rulesets/<ID> --input /tmp/rs-patch.json \
  --jq '{name, enforcement, bypass_actors}'

# 3d. Verify with the guard in team mode:
python3 scripts/check-develop-ruleset.py --team-mode
```

Expected: the guard prints `pull_request rule requires a human approving
review (team mode)` and `RESULT: PASS`.

## Step 4 — Make team mode the enforced default

Change the ruleset-guard workflow invocation so CI *fails* when the live
ruleset reverts to approval count 0:

- In `.github/workflows/ruleset-guard.yml`: add `--team-mode` to the
  `scripts/check-develop-ruleset.py` invocation(s).
- In `scripts/check-develop-ruleset.py`: flip the default
  (`team_mode` default `True` in `evaluate`/`run_live`) or hardcode the
  mode, so a caller that forgets the flag cannot silently re-assert the
  solo policy.
- Same PR: adjust the self-test expectations (the `solo mode: approval
  count 1 fails` scenario becomes the team-mode default case).

Push that as a PR, get it approved by the second maintainer — this PR is
the first one to travel through the **new** gate and proves it end to end.

## Step 5 — Post-flip verification

- [ ] `gh pr checks <that PR>` green, merge via squash as usual.
- [ ] `python3 scripts/check-develop-ruleset.py --team-mode` → PASS.
- [ ] Dependabot PRs still auto-merge (approval comes from the
      `dependabot-auto-merge` workflow or a maintainer; verify one cycle).
      If the workflow's approval path cannot satisfy the new count, decide
      explicitly: a maintainer approves Dependabot PRs by hand, or the
      workflow gains a dedicated `environment` approval — never a ruleset
      bypass actor.
- [ ] Update `docs/clean-desk.md` transition checklist (all boxes).
- [ ] Announce in the issue that tracks team enablement; done.

## Rollback (only if the gate misbehaves)

Re-run step 3 with the count set back to `0`, revert the `--team-mode`
default in the guard, and file an issue describing what blocked a legitimate
merge. Rollback is itself reviewed: land it as a PR if the gate still lets
PRs through; if nothing can merge at all, apply the rollback directly and
document the exception in the issue (the no-bypass-actors invariant stays
untouched in every scenario — it is what keeps the rollback honest).
