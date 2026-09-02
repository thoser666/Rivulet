#!/usr/bin/env bash
set -euo pipefail

# Local smoke test for the bilingual wiki sync workflow.
#
# Verifies three things against the local wiki working clone
# (.freebuff-rivulet-wiki by default):
#   1. the clone exists and is in sync with its remote (no local commits, no
#      pushed-but-missing changes, clean working tree);
#   2. every English page has a -de pair and the language-switch links are
#      present (check-wiki-translations.py);
#   3. the i18n drift check of sync-wiki-translations.py passes in --check mode.
#   4. every wiki link resolves: interwiki pages + anchors, repo-doc files
#      + heading anchors, and external URLs are reachable (network required;
#      export WIKI_LINK_AUDIT_EXTRA=--skip-external for offline runs).
#
# Unlike the scheduled CI job it never fetches or pushes; it only inspects the
# clone you already have. Clone it once with:
#   git clone https://github.com/thoser666/Rivulet.wiki.git .freebuff-rivulet-wiki
#
# Exit codes: 0 = all checks passed; 1 = a check failed; 2 = environment or
# clone misconfiguration (missing clone, missing scripts, missing git repo).

CLONE="${WIKI_CLONE_DIR:-.freebuff-rivulet-wiki}"
FAILURES=0

fail() {
  echo "FAIL: $*" >&2
  FAILURES=$((FAILURES + 1))
}

# --- prerequisites ---------------------------------------------------------
for script in scripts/check-wiki-translations.py scripts/sync-wiki-translations.py; do
  if [[ ! -f "$script" ]]; then
    echo "FAIL: required script $script not found" >&2
    exit 2
  fi
done

if [[ ! -d "$CLONE/.git" ]]; then
  echo "FAIL: wiki clone $CLONE is missing (no .git directory)" >&2
  echo "      clone it first:" >&2
  echo "      git clone https://github.com/thoser666/Rivulet.wiki.git $CLONE" >&2
  exit 2
fi

python3 --version >/dev/null 2>&1 || { echo "FAIL: python3 is required" >&2; exit 2; }

echo "== Wiki sync smoke test =="
echo "clone:  ${CLONE}"
echo "remote: $(git -C "$CLONE" remote get-url origin 2>/dev/null || echo '(none)')"
echo

# --- 1. remote sync state --------------------------------------------------
echo "-- 1. remote sync state"
git -C "$CLONE" fetch --quiet origin 2>/dev/null || {
  fail "could not fetch origin from $CLONE (offline? network blocked?)"
}

local_head="$(git -C "$CLONE" rev-parse HEAD 2>/dev/null || true)"
remote_head="$(git -C "$CLONE" rev-parse @{u} 2>/dev/null || true)"

if [[ -z "$remote_head" ]]; then
  fail "$CLONE has no upstream branch (run: git -C $CLONE branch --set-upstream-to=origin/master)"
elif [[ "$local_head" == "$remote_head" ]]; then
  echo "ok: HEAD matches upstream ($(echo "$local_head" | cut -c1-9))"
else
  count="$(git -C "$CLONE" rev-list --left-right --count @{u}...HEAD 2>/dev/null || echo '?')"
  fail "clone is not in sync with upstream ($local_head vs $remote_head; ahead/behind: $count)"
fi

if ! git -C "$CLONE" diff --quiet; then
  fail "$CLONE has uncommitted working-tree changes (git status shows modifications)"
elif [[ "$(git -C "$CLONE" status --porcelain | wc -l | tr -d ' ')" != "0" ]]; then
  fail "$CLONE has unstaged or untracked files; work tree is not clean"
else
  echo "ok: working tree is clean"
fi

echo

# --- 2. language pairs (check-wiki-translations.py) ------------------------
echo "-- 2. language pairs"
if python3 scripts/check-wiki-translations.py "$CLONE"; then
  echo "ok: every English page has a -de pair with language-switch links"
else
  fail "check-wiki-translations.py reported missing pairs or language links"
fi
echo

# --- 3. i18n drift check (sync-wiki-translations.py --check) ---------------
echo "-- 3. i18n drift check"
if python3 scripts/sync-wiki-translations.py --check "$CLONE"; then
  echo "ok: no translation drift (switch links consistent)"
else
  fail "sync-wiki-translations.py --check reported drift that needs syncing"
fi
echo

# --- 4. full wiki link audit (audit-wiki-links.py) -------------------------
echo "-- 4. wiki link audit (interwiki + repo-docs + external + repo-doc wiki refs)"
if python3 scripts/audit-wiki-links.py "$CLONE" --check-repo-docs ${WIKI_LINK_AUDIT_EXTRA:-}; then
  echo "ok: all wiki links resolve (pages, anchors, URLs)"
else
  fail "audit-wiki-links.py found broken wiki links"
fi
echo

# --- summary ---------------------------------------------------------------
if [[ "$FAILURES" -eq 0 ]]; then
  echo "Wiki sync smoke test PASSED"
  exit 0
fi

echo "Wiki sync smoke test FAILED: ${FAILURES} check(s)" >&2
exit 1