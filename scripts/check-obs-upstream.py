#!/usr/bin/env python3
"""Check OBS release-note candidates AND Rivulet open issues against the vision."""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sys
import tempfile
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "scripts" / "obs-features.json"
VISION = ROOT / "scripts" / "vision-criteria.json"
STATE = ROOT / "scripts" / ".obs-upstream-state.json"
DOC = ROOT / "docs" / "obs-vision-candidates.md"
COMMUNITY_DOC = ROOT / "docs" / "community-wish-candidates.md"
API = "https://api.github.com/repos/obsproject/obs-studio/releases/latest"
ISSUES_API = (
    "https://api.github.com/repos/{repo}/issues"
    "?state={state}&per_page=100&sort=updated&direction=desc&page={page}"
)
# The dedupe must also see closed canonical threads, so its fetch walks a
# bounded number of pages of every issue state, most recently updated first.
TRACKED_ISSUE_PAGES = 3
# Only candidates that carry a product decision may file a new issue; a
# not-aligned upstream note stays in the report instead of the tracker.
CREATE_DECISIONS = {"strong-fit", "review"}
REPO_ISSUES_API = "https://api.github.com/repos/{repo}/issues"
# One issue per OBS release-note candidate. The title carries the upstream
# line, optionally followed by the upstream author in brackets and the vision
# decision; the dedupe key strips all of that so a candidate is recognised no
# matter which of those decorations a previous run wrote.
CANDIDATE_PREFIX = "OBS: "
CANDIDATE_MARKER = "<!-- obs-upstream-candidate -->"
DECISION_SUFFIX = re.compile(
    r"\s*—\s*(?:strong-fit|review|not-aligned)\s*\([^)]*\)\s*$", re.IGNORECASE
)
TRAILING_AUTHOR = re.compile(r"\s*\[[^\[\]]*\]\s*$")
FEATURE_WORDS = re.compile(r"\b(add|added|introduce|introduced|support|supports|new|feature|implemented)\b", re.I)
# Issue labels that mark a request as an open product wish (triage only).
# "enhancement"/"epic" deliberately are NOT wishes: in this repository they
# mean roadmap-tracked milestone work, which the sweep lists separately
# instead of re-triaging it every week.
FEATURE_LABELS = {"feature", "idea", "wish"}
ROADMAP_LABELS = {"enhancement", "epic"}
BUG_LABELS = {"bug", "bugfix", "fix", "regression"}
COMMUNITY_START = "<!-- COMMUNITY-WISH-CANDIDATES:START -->"
COMMUNITY_END = "<!-- COMMUNITY-WISH-CANDIDATES:END -->"
# Single-word catalog aliases that are specific enough to signal a covered
# roadmap feature in an issue title ("mkv", "vst", "webcam" …). Generic
# words like "streaming"/"scenes"/"window" would tag bug reports and unrelated
# wishes as "already represented", so they are only used on OBS release lines,
# never on issue titles.
DISTINCTIVE_ISSUE_TERMS = {
    "webcam", "mkv", "mp4", "encoding", "rtmp", "webrtc", "whip",
    "stinger", "projector", "lut", "chroma", "ducking", "ndi", "vst",
    "midi", "i18n", "locale", "remux", "undo", "copy/paste", "per-app",
    "obs-websocket", "streamdeck", "hotkey", "hotkeys", "mixer", "volume",
    "noise suppression", "chroma key", "replay buffer", "virtual camera",
    "scene collections", "studio mode", "adaptive bitrate", "audio tracks",
    "file management", "video capture device", "copy scene items",
}


def issue_catalog_terms() -> list[str]:
    """Roadmap-catalog terms that are reliable on issue titles: multi-word
    aliases and distinctive single tokens. Generic aliases stay OBS-line-only.
    """
    terms = []
    for item in load_catalog():
        for alias in [item.get("id", ""), *item.get("aliases", [])]:
            alias = alias.lower().strip()
            if not alias:
                continue
            if " " in alias or alias in DISTINCTIVE_ISSUE_TERMS:
                terms.append(alias)
    return terms


def load_catalog() -> list[dict]:
    return json.loads(CATALOG.read_text(encoding="utf-8"))["features"]


def load_vision() -> dict:
    return json.loads(VISION.read_text(encoding="utf-8"))


def previous_tag_from(path: Path) -> str | None:
    """Read the last checked release tag from a state file, if any.

    The state file is a runtime artifact (restored from the actions cache in
    CI, never committed), so a missing or corrupt file simply means "no
    previous check" — never a hard failure.
    """
    if not path.exists():
        return None
    try:
        tag = json.loads(path.read_text(encoding="utf-8")).get("tag_name")
    except (json.JSONDecodeError, OSError):
        return None
    return tag or None


def persist_state(release: dict, path: Path = STATE) -> None:
    """Remember the checked release so the next run can report the delta.

    Written atomically (temp file + rename) so a crash mid-write cannot
    corrupt the state the next run reads back. The workflow persists this
    file across runs through the actions cache.
    """
    payload = {
        "tag_name": release.get("tag_name", "unknown"),
        "checked_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    tmp.replace(path)


def fetch_release() -> dict:
    request = urllib.request.Request(API, headers={"Accept": "application/vnd.github+json", "User-Agent": "rivulet-obs-check"})
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.load(response)


def fetch_issues(repo: str, state: str = "open", pages: int = 1) -> list[dict]:
    """Fetch issues in one state, following at most `pages` pages.

    The issues API returns PRs too; they are filtered out downstream by the
    `pull_request` key. Uses the GITHUB_TOKEN when available so private/fork
    runs work identically.
    """
    headers = {"Accept": "application/vnd.github+json", "User-Agent": "rivulet-obs-check"}
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    issues: list[dict] = []
    for page in range(1, pages + 1):
        url = ISSUES_API.format(repo=repo, state=state, page=page)
        request = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(request, timeout=30) as response:
            batch = json.load(response)
        issues.extend(batch)
        if len(batch) < 100:
            break
    return issues


def fetch_open_issues(repo: str) -> list[dict]:
    return fetch_issues(repo)


def fetch_tracked_issues(repo: str) -> list[dict]:
    """Open *and* closed issues, so the dedupe sees a canonical thread either way."""
    return fetch_issues(repo, state="all", pages=TRACKED_ISSUE_PAGES)


def candidate_key(title: str) -> str:
    """Normalized identity of an OBS candidate issue title.

    Drops the `OBS: ` prefix, a trailing vision-decision suffix and a trailing
    `[author]` tag, then collapses everything to lowercase words. This is the
    key the sweep compares, so the same upstream line matches an issue created
    by an earlier run even if that run appended the author or decision.
    """
    text = title.strip()
    if text.lower().startswith(CANDIDATE_PREFIX.lower()):
        text = text[len(CANDIDATE_PREFIX):]
    text = DECISION_SUFFIX.sub("", text)
    text = TRAILING_AUTHOR.sub("", text.strip())
    return " ".join(re.sub(r"[^a-z0-9]+", " ", text.lower()).split())


def candidate_issue_title(line: str) -> str:
    return f"{CANDIDATE_PREFIX}{line}"


def candidate_issue_body(tag: str, line: str, decision: str, matches: list[str]) -> str:
    return "\n".join([
        CANDIDATE_MARKER,
        f"Upstream: OBS Studio `{tag}` release notes.",
        f"Vision decision: **{decision}** ({', '.join(matches) or 'none'})",
        "",
        "Upstream release note:",
        "",
        f"> {line}",
        "",
        "Maintained by `scripts/check-obs-upstream.py`: when the same upstream title is seen",
        "again this issue is updated in place instead of filing a duplicate.",
        "",
    ])


def issue_state(issue: dict) -> str:
    return str(issue.get("state") or "open").lower()


def find_candidate_issue(issues: list[dict], line: str) -> dict | None:
    """The non-PR issue that already tracks this upstream candidate.

    Matching is on the normalized title only, so comments and labels on the
    existing thread survive and stay attached to the same upstream change.
    Closed threads match as well: a decision made once must not come back as a
    new issue every week. An open match wins over a closed one.
    """
    key = candidate_key(line)
    if not key:
        return None
    closed = None
    for issue in issues:
        if issue_is_pr(issue):
            continue
        if candidate_key(issue.get("title", "")) != key:
            continue
        if issue_state(issue) == "closed":
            closed = closed or issue
            continue
        return issue
    return closed


def github_request(token: str, method: str, url: str, payload: dict | None = None) -> dict:
    data = json.dumps(payload).encode("utf-8") if payload is not None else None
    request = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "rivulet-obs-check",
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        body = response.read()
    return json.loads(body) if body else {}


def sync_candidate_issues(
    release: dict,
    issues: list[dict],
    repo: str,
    request=None,
    tag: str | None = None,
) -> tuple[list[int], list[int]]:
    """Upsert one issue per OBS candidate, keyed on the upstream title.

    Returns (created, updated, kept_closed) issue numbers. `issues` is the
    tracked list (open and closed) and is mutated with the issues created here
    so a repeated candidate in the same release notes cannot be filed twice.
    Covered candidates (already in the roadmap catalog) are skipped, as in the
    report. A closed thread that already tracks the title is reported instead
    of being reopened, updated, or filed again, and only strong-fit/review
    candidates create a new issue.
    """
    if request is None:
        token = os.environ.get("GITHUB_TOKEN", "")
        if not token:
            raise RuntimeError("--sync-issues needs GITHUB_TOKEN")

        def request(method: str, url: str, payload: dict | None = None) -> dict:
            return github_request(token, method, url, payload)

    vision = load_vision()
    tag = tag or release.get("tag_name", "unknown")
    endpoint = REPO_ISSUES_API.format(repo=repo)
    created: list[int] = []
    updated: list[int] = []
    kept_closed: list[int] = []
    for line in candidate_lines(release.get("body", "")):
        covered, _ = classify([line], load_catalog())
        if covered:
            continue
        decision, matches = vision_fit(line, vision)
        body = candidate_issue_body(tag, line, decision, matches)
        existing = find_candidate_issue(issues, line)
        if existing is not None:
            if issue_state(existing) == "closed":
                # Reopening would undo a maintainer decision; refiling would put
                # the same upstream change back in the queue every week.
                kept_closed.append(existing["number"])
                continue
            request("PATCH", f"{endpoint}/{existing['number']}", {"body": body})
            updated.append(existing["number"])
            continue
        if decision not in CREATE_DECISIONS:
            continue
        issue = request("POST", endpoint, {"title": candidate_issue_title(line), "body": body})
        created.append(issue.get("number"))
        issues.append({"number": issue.get("number"), "title": candidate_issue_title(line), "state": "open"})
    return created, updated, kept_closed


def issue_is_pr(issue: dict) -> bool:
    return bool(issue.get("pull_request"))


def issue_labels(issue: dict) -> set[str]:
    return {label.get("name", "").lower() for label in issue.get("labels", [])}


def is_feature_request(issue: dict) -> bool:
    """Triage: an open, non-PR issue counts as a feature wish when it carries
    a feature-ish label or its title reads like a feature addition. Bug
    reports and pure housekeeping are deliberately excluded — this is a
    product-wish sweep, not an issue tracker mirror.
    """
    labels = issue_labels(issue)
    if labels & BUG_LABELS:
        return False
    if labels & FEATURE_LABELS:
        return True
    title = issue.get("title", "")
    if FEATURE_WORDS.search(title):
        return True
    # Roadmap-catalog wording in the title (e.g. "Replay buffer while
    # streaming") marks a wish too — analyze_issues then reports it as
    # "already represented" instead of silently dropping it.
    lower = title.lower()
    return any(term in lower for term in issue_catalog_terms())


def issue_view(issue: dict, decision: str, matches: list[str]) -> dict:
    return {
        "number": issue.get("number"),
        "title": issue.get("title", ""),
        "url": issue.get("html_url", ""),
        "decision": decision,
        "matches": matches,
        "text": f"{issue.get('title', '')}\n{issue.get('body') or ''}",
    }


def is_roadmap_tracked(issue: dict) -> bool:
    """Milestone-owned work (enhancement/epic labels, e.g. the [Mx] backlog
    issues) is excluded from the wish review — it already has a roadmap home.
    """
    return bool(issue_labels(issue) & ROADMAP_LABELS)


def candidate_lines(body: str) -> list[str]:
    return list(dict.fromkeys(line.strip(" -*\t") for line in body.splitlines() if line.strip(" -*\t") and FEATURE_WORDS.search(line)))


def classify(lines: list[str], catalog: list[dict]) -> tuple[list[str], list[str]]:
    covered, review = [], []
    for line in lines:
        if any(alias.lower() in line.lower() for item in catalog for alias in [item.get("id", ""), *item.get("aliases", [])]):
            covered.append(line)
        else:
            review.append(line)
    return covered, review


def vision_fit(line: str, vision: dict) -> tuple[str, list[str]]:
    text = line.lower()
    matched = [criterion["id"] for criterion in vision["criteria"] if any(re.search(rf"\b{re.escape(keyword.lower())}\b", text) for keyword in criterion["keywords"])]
    decision = vision["decision"]
    if len(matched) >= decision["strong_fit_minimum"]:
        return "strong-fit", matched
    if len(matched) >= decision["review_minimum"]:
        return "review", matched
    return "not-aligned", matched


def strong_fit_rows(lines: list[str], vision: dict) -> list[tuple[str, list[str]]]:
    rows = []
    for line in lines:
        label, matches = vision_fit(line, vision)
        if label == "strong-fit":
            rows.append((line, matches))
    return rows


def candidate_doc_fragment(release: dict, rows: list[tuple[str, list[str]]]) -> str:
    if not rows:
        return "<!-- No new strong-fit candidates detected in the latest checked release. -->"
    version = release.get("tag_name", "unknown")
    lines = [f"### {version}", "", "| Candidate | Matching vision pillars |", "| --- | --- |"]
    lines.extend(f"| {line.replace('|', '\\|')} | {', '.join(matches)} |" for line, matches in rows)
    return "\n".join(lines)


def update_candidate_doc(release: dict, rows: list[tuple[str, list[str]]]) -> None:
    if not DOC.exists():
        return
    text = DOC.read_text(encoding="utf-8")
    start = "<!-- OBS-VISION-CANDIDATES:START -->"
    end = "<!-- OBS-VISION-CANDIDATES:END -->"
    if start not in text or end not in text:
        raise ValueError(f"{DOC}: candidate markers missing")
    fragment = candidate_doc_fragment(release, rows)
    # The END marker must be written back too — dropping it (as an earlier
    # version did) leaves the doc with only a START marker, which makes the
    # next run fail with "candidate markers missing" and silently empties
    # the weekly report (masked by continue-on-error in the workflow).
    updated = text.split(start)[0] + start + "\n" + fragment + "\n" + end + text.split(end, 1)[1]
    DOC.write_text(updated, encoding="utf-8")


def analyze_issues(issues: list[dict]) -> tuple[list[dict], list[dict], list[dict], list[dict]]:
    """Classify open issues. Returns (review, covered, strong, roadmap):
    review rows carry the advisory vision decision for genuine wishes, covered
    wishes are already represented in the roadmap catalog, strong rows are the
    review-priority subset, and roadmap rows are milestone-tracked work
    (enhancement/epic) that is excluded from the wish review but still shown.
    """
    vision = load_vision()
    review, covered, strong, roadmap = [], [], [], []
    for issue in issues:
        labels = issue_labels(issue)
        if issue_is_pr(issue) or labels & BUG_LABELS:
            continue
        # Roadmap-owned milestone work (enhancement/epic) is excluded from the
        # wish review even when its title reads like an addition; an explicit
        # feature/idea/wish label marks a genuine open wish instead.
        if labels & FEATURE_LABELS:
            wish = True
        elif labels & ROADMAP_LABELS:
            roadmap.append(issue_view(issue, "", []))
            continue
        else:
            wish = is_feature_request(issue)
        if not wish:
            continue
        text = f"{issue.get('title', '')}\n{issue.get('body') or ''}"
        # Covered detection uses the title only and only distinctive terms:
        # bodies and generic single words ("scenes", "window", "streaming" …)
        # would otherwise hide an unrelated wish behind an "already
        # represented" verdict.
        title_lower = issue.get("title", "").lower()
        if any(term in title_lower for term in issue_catalog_terms()):
            covered.append(issue_view(issue, "covered", []))
            continue
        decision, matches = vision_fit(text, vision)
        view = issue_view(issue, decision, matches)
        review.append(view)
        if decision == "strong-fit":
            strong.append(view)
    return review, covered, strong, roadmap


def community_fragment(rows: list[dict]) -> str:
    if not rows:
        return "<!-- No new strong-fit community wishes detected in the latest sweep. -->"
    lines = [f"### Open-issue sweep — {datetime.now(timezone.utc).date().isoformat()}", "", "| Issue | Matching vision pillars |", "| --- | --- |"]
    for row in rows:
        title = row["title"].replace("|", "\\|")
        link = f"[#{row['number']} {title}]({row['url']})" if row["url"] else f"#{row['number']} {title}"
        lines.append(f"| {link} | {', '.join(row['matches'])} |")
    return "\n".join(lines)


def update_community_doc(rows: list[dict], doc: Path = COMMUNITY_DOC) -> None:
    if not doc.exists():
        return
    text = doc.read_text(encoding="utf-8")
    if COMMUNITY_START not in text or COMMUNITY_END not in text:
        raise ValueError(f"{doc}: community candidate markers missing")
    fragment = community_fragment(rows)
    updated = text.split(COMMUNITY_START)[0] + COMMUNITY_START + "\n" + fragment + "\n" + COMMUNITY_END + text.split(COMMUNITY_END, 1)[1]
    doc.write_text(updated, encoding="utf-8")


def review_checklist(release: dict, issues: list[dict] | None) -> str:
    """Actionable review checklist for the weekly GitHub issue: every OBS
    release-note candidate and community wish that still needs a maintainer
    decision becomes one unchecked checkbox. Already-represented and
    roadmap-tracked items are deliberately not checklist rows.
    """
    vision = load_vision()
    items = []
    for line in candidate_lines(release.get("body", "")):
        covered, _ = classify([line], load_catalog())
        if covered:
            continue
        label, matches = vision_fit(line, vision)
        items.append(f"- [ ] OBS: {line.replace('|', '\\|')} — {label} ({', '.join(matches) or 'none'})")
    if issues is not None:
        review, _, _, _ = analyze_issues(issues)
        for row in review:
            link = f"[#{row['number']} {row['title']}]({row['url']})" if row["url"] else f"#{row['number']} {row['title']}"
            items.append(f"- [ ] Wish: {link} — {row['decision']} ({', '.join(row['matches']) or 'none'})")
    if not items:
        return "No open review items this week — nothing to triage."
    return "\n".join(items)


def issues_report(issues: list[dict], fixture: bool = False) -> str:
    review, covered, strong, roadmap = analyze_issues(issues)
    open_count = sum(1 for i in issues if not issue_is_pr(i))
    result = [
        "## Rivulet open issues — vision triage",
        "",
        f"Source: {'fixture' if fixture else 'GitHub API'}",
        f"Open issues (non-PR): {open_count}",
        f"Feature-request candidates: {len(review) + len(covered)}",
        f"Strong fit: {len(strong)}",
        f"Roadmap-tracked (excluded from wish review): {len(roadmap)}",
        "",
    ]
    if review:
        result += ["### Needs review — vision fit", "", "| Issue | Vision decision | Matching pillars |", "| --- | --- | --- |"]
        for row in review:
            title = row["title"].replace("|", "\\|")
            link = f"[#{row['number']} {title}]({row['url']})" if row["url"] else f"#{row['number']} {title}"
            result.append(f"| {link} | **{row['decision']}** | {', '.join(row['matches']) or 'none'} |")
        result.append("")
    if covered:
        result += ["### Already represented by the roadmap catalog"]
        for row in covered:
            title = row["title"].replace("|", "\\|")
            link = f"[#{row['number']} {title}]({row['url']})" if row["url"] else f"#{row['number']} {title}"
            result.append(f"- {link}")
        result.append("")
    if roadmap:
        result += ["### Roadmap-tracked (excluded — already milestone work)"]
        for row in roadmap:
            title = row["title"].replace("|", "\\|")
            link = f"[#{row['number']} {title}]({row['url']})" if row["url"] else f"#{row['number']} {title}"
            result.append(f"- {link}")
        result.append("")
    if not review and not covered:
        result.append("No open feature wishes that are not already covered.")
        result.append("")
    result += [
        "Vision decisions are advisory. Strong-fit wishes are copied to `docs/community-wish-candidates.md` for maintainer review; they are not automatically added to the roadmap.",
        "",
        "Vision criteria: `scripts/vision-criteria.json`.",
    ]
    return "\n".join(result)


def report(release: dict, fixture: bool = False) -> str:
    vision = load_vision()
    lines = candidate_lines(release.get("body", ""))
    covered, review = classify(lines, load_catalog())
    previous = previous_tag_from(STATE)
    result = [f"## OBS upstream check — {release.get('tag_name', 'unknown')}", "", f"Source: {'fixture' if fixture else 'GitHub API'}", f"Previous checked release: {previous or 'none'}", f"Feature-note candidates: {len(lines)}", ""]
    if review:
        result += ["### Needs review — vision fit", "", "| Candidate | Vision decision | Matching pillars |", "| --- | --- | --- |"]
        for line in review:
            label, matches = vision_fit(line, vision)
            result.append(f"| {line.replace('|', '\\|')} | **{label}** | {', '.join(matches) or 'none'} |")
        result.append("")
    if covered:
        result += ["### Already represented", *[f"- {line}" for line in covered], ""]
    result += ["Vision decisions are advisory. Strong-fit candidates are copied to `docs/obs-vision-candidates.md` for maintainer review; they are not automatically added to the roadmap.", "", "Vision criteria: `scripts/vision-criteria.json`."]
    return "\n".join(result)


def self_test() -> None:
    fixture = {"tag_name": "30.0.0", "body": "- Added a new automated GPU capture\n- Added a new emoji\n- Supports multitrack video"}
    vision = load_vision()
    output = report(fixture, fixture=True)
    assert "automated GPU capture" in output and "strong-fit" in output and "not-aligned" in output
    assert strong_fit_rows(candidate_lines(fixture["body"]), vision)
    label, matches = vision_fit("New deterministic WebGPU stream API", vision)
    assert label == "strong-fit" and {"deterministic", "modern-rendering", "streamer-value"}.issubset(matches)
    # State round-trip: persist a checked release, then the delta tracking
    # must report it as the previous tag on the next run, and a corrupt or
    # missing state file must degrade to "none" instead of crashing.
    tmp_dir = tempfile.mkdtemp(prefix="obs-state-test-")
    try:
        state = Path(tmp_dir) / "state.json"
        assert previous_tag_from(state) is None, "missing state must read as no previous tag"
        persist_state(fixture, state)
        assert previous_tag_from(state) == "30.0.0", "persisted tag must be read back"
        state.write_text("{ not json", encoding="utf-8")
        assert previous_tag_from(state) is None, "corrupt state must degrade to none"
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)

    # Community-wish sweep: feature-request issues are triaged with the same
    # vision criteria, roadmap-tracked milestone issues are listed separately,
    # and the doc updater must preserve both markers.
    issues = [
        {"number": 1, "title": "Add a deterministic headless WebGPU scene API", "body": "Allow scripts to switch scenes over the API without the GUI", "html_url": "https://github.com/x/y/issues/1"},
        {"number": 2, "title": "Crash when streaming to Twitch", "body": "Happens every hour", "html_url": "https://github.com/x/y/issues/2"},
        {"number": 3, "title": "Scene collections import from OBS", "body": "OBS already has this", "html_url": "https://github.com/x/y/issues/3", "labels": [{"name": "feature"}]},
        {"number": 4, "title": "[M6] Multi-platform restream", "body": "Milestone backlog item", "html_url": "https://github.com/x/y/issues/4", "labels": [{"name": "enhancement"}]},
    ]
    review, covered, strong, roadmap = analyze_issues(issues)
    assert [row["number"] for row in review] == [1], "WebGPU issue must need review"
    assert [row["number"] for row in covered] == [3], "scene-collections wish is already represented"
    assert [row["number"] for row in roadmap] == [4], "enhancement milestone work must be listed as roadmap-tracked"
    assert strong and strong[0]["number"] == 1, "WebGPU issue must be a strong fit"
    issue_output = issues_report(issues, fixture=True)
    assert "## Rivulet open issues — vision triage" in issue_output
    assert "Crash when streaming to Twitch" not in issue_output, "bug reports are not wishes"
    assert "Already represented" in issue_output and "Scene collections" in issue_output
    assert "Roadmap-tracked" in issue_output and "Multi-platform restream" in issue_output
    tmp_dir = tempfile.mkdtemp(prefix="community-doc-test-")
    try:
        community = Path(tmp_dir) / "community.md"
        community.write_text(
            "# C\n" + COMMUNITY_START + "\n<!-- stale -->\n" + COMMUNITY_END + "\n",
            encoding="utf-8",
        )
        update_community_doc(strong, community)
        updated = community.read_text(encoding="utf-8")
        assert updated.count(COMMUNITY_START) == 1 and updated.count(COMMUNITY_END) == 1
        assert "#1 Add a deterministic headless WebGPU scene API" in updated
        assert "<!-- stale -->" not in updated
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)

    # Review checklist: actionable rows as checkboxes, no covered/bug rows.
    checklist = review_checklist(fixture, issues)
    assert "- [ ] OBS: Added a new automated GPU capture" in checklist
    assert "- [ ] Wish: [#1 Add a deterministic headless WebGPU scene API]" in checklist
    assert "Scene collections" not in checklist, "covered wishes are not checklist rows"
    assert "Crash when streaming" not in checklist, "bug reports are not checklist rows"
    # Candidate-issue dedupe: an existing open issue for the same upstream
    # title is updated in place, only unseen titles create an issue, and a
    # second sweep over the same release notes must not file anything again.
    canonical = (
        "OBS: Changed new capture devices to use fallback frame rate by default"
        " [PatTheMav] — review (streamer-value)"
    )
    bare = "Changed new capture devices to use fallback frame rate by default"
    assert candidate_key(canonical) == candidate_key(bare), "title decorations must not defeat the dedupe key"
    assert candidate_key(bare) == "changed new capture devices to use fallback frame rate by default"
    open_issues = [
        {"number": 7, "title": canonical},
        {"number": 8, "title": "Unrelated wish", "labels": [{"name": "feature"}]},
        {"number": 9, "title": "OBS: Never filed before", "pull_request": {"url": "https://x"}},
    ]
    assert find_candidate_issue(open_issues, bare)["number"] == 7, "the existing issue must be found"
    assert find_candidate_issue(open_issues, "Never filed before") is None, "pull requests are not tracked issues"

    calls: list[tuple[str, str, dict]] = []

    def fake_request(method: str, url: str, payload: dict | None = None) -> dict:
        calls.append((method, url, payload or {}))
        return {"number": 99} if method == "POST" else {}

    sync_release = {
        "tag_name": "31.0.0",
        "body": f"- {bare}\n- Added a new hardware encoder preset",
    }
    created, updated, kept_closed = sync_candidate_issues(sync_release, open_issues, "x/y", request=fake_request)
    assert updated == [7], "the seen title must update the existing issue instead of filing a duplicate"
    assert created == [99], "only the unseen title may create an issue"
    assert kept_closed == [], "nothing is closed in this fixture"
    assert [call[0] for call in calls] == ["PATCH", "POST"]
    assert calls[0][1].endswith("/issues/7")
    assert calls[1][2]["title"] == "OBS: Added a new hardware encoder preset"
    assert CANDIDATE_MARKER in calls[0][2]["body"] and "**review**" in calls[0][2]["body"]
    calls.clear()
    created_again, updated_again, kept_again = sync_candidate_issues(sync_release, open_issues, "x/y", request=fake_request)
    assert created_again == [], "a second sweep of the same release notes must not file duplicates"
    assert updated_again == [7, 99] and kept_again == []

    # Closed canonical threads: an already-decided candidate must not return as
    # a new issue, and it must not be reopened or rewritten either. An open
    # match still wins over a closed one when both exist.
    both = [{"number": 11, "state": "closed", "title": bare}, {"number": 12, "title": canonical}]
    assert find_candidate_issue(both, bare)["number"] == 12, "an open thread wins over a closed one"
    only_closed = [{"number": 13, "state": "closed", "title": canonical}]
    assert find_candidate_issue(only_closed, bare)["number"] == 13, "closed threads must match the dedupe key"
    calls.clear()
    created_closed, updated_closed, kept_closed = sync_candidate_issues(
        {"tag_name": "31.1.0", "body": f"- {bare}"}, only_closed, "x/y", request=fake_request
    )
    assert (created_closed, updated_closed, kept_closed) == ([], [], [13])
    assert calls == [], "a closed canonical thread is neither reopened, updated, nor refiled"

    # Only strong-fit/review candidates file a new issue; a not-aligned note
    # stays in the report instead of growing the tracker.
    calls.clear()
    not_aligned = sync_candidate_issues(
        {"tag_name": "31.2.0", "body": "- Added a new emoji picker"}, [], "x/y", request=fake_request
    )
    assert not_aligned == ([], [], [])
    assert calls == [], "not-aligned candidates must not create an issue"

    # Token path: without an injected transport the sync must talk to the API
    # through github_request with GITHUB_TOKEN, and a missing token must be a
    # loud error rather than a silent no-op.
    saved_token = os.environ.pop("GITHUB_TOKEN", None)
    try:
        try:
            sync_candidate_issues(sync_release, [], "x/y")
        except RuntimeError:
            pass
        else:
            raise AssertionError("issue sync without GITHUB_TOKEN must fail")
        os.environ["GITHUB_TOKEN"] = "self-test-token"
        real_request = globals()["github_request"]
        calls.clear()
        globals()["github_request"] = lambda token, method, url, payload=None: (
            calls.append((method, url, payload or {})) or ({"number": 99} if method == "POST" else {})
        )
        try:
            token_created, token_updated, token_closed = sync_candidate_issues(sync_release, [], "x/y")
        finally:
            globals()["github_request"] = real_request
        assert token_created == [99, 99] and token_updated == [] and token_closed == []
        assert [call[0] for call in calls] == ["POST", "POST"], "every candidate goes through the API"
    finally:
        os.environ.pop("GITHUB_TOKEN", None)
        if saved_token is not None:
            os.environ["GITHUB_TOKEN"] = saved_token
    print("OK: OBS upstream + community wish checker self-test")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--issues-file", type=Path, help="JSON list of open issues (overrides the GitHub fetch; used with --fixture for offline runs)")
    parser.add_argument("--no-issues", action="store_true", help="skip the Rivulet open-issue sweep entirely")
    parser.add_argument("--issues-repo", default=os.environ.get("ISSUES_REPO", "thoser666/Rivulet"))
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--update-doc", action="store_true", help="update docs/obs-vision-candidates.md and docs/community-wish-candidates.md with strong-fit candidates")
    parser.add_argument("--checklist-file", type=Path, help="write the review checklist (for the weekly GitHub issue) to this file")
    parser.add_argument(
        "--sync-issues",
        action="store_true",
        help=(
            "upsert one open issue per OBS candidate, deduplicated by upstream title "
            "(updates an existing issue instead of filing a duplicate; needs GITHUB_TOKEN)"
        ),
    )
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    try:
        release = json.loads(args.fixture.read_text(encoding="utf-8")) if args.fixture else fetch_release()
        if args.issues_file:
            issues = json.loads(args.issues_file.read_text(encoding="utf-8"))
        elif args.no_issues or args.fixture:
            # Fixture runs stay offline unless an explicit issues file is given.
            issues = None
        else:
            issues = fetch_open_issues(args.issues_repo)
        if args.sync_issues:
            # Dedupe lives here so every caller (scheduled run, manual dispatch,
            # local replay) goes through the same title normalisation. The
            # tracked fetch reads closed issues too: a canonical thread that was
            # closed must not reappear as a new issue on the next run.
            tracked = issues if args.issues_file else fetch_tracked_issues(args.issues_repo)
            created, updated, kept_closed = sync_candidate_issues(release, tracked, args.issues_repo)
            print(
                f"OBS candidate issues: {len(created)} created, {len(updated)} updated, "
                f"{len(kept_closed)} left closed (deduplicated by upstream title)"
            )
            return 0
        if args.update_doc:
            rows = strong_fit_rows(candidate_lines(release.get("body", "")), load_vision())
            update_candidate_doc(release, rows)
            if issues is not None:
                _, _, strong, _ = analyze_issues(issues)
                update_community_doc(strong)
        if not args.fixture:
            # Persist the tag so the next run can show the delta; fixture
            # runs (tests/demos) must never touch the real state file.
            persist_state(release)
        if args.checklist_file:
            args.checklist_file.write_text(
                review_checklist(release, issues),
                encoding="utf-8",
            )
        sections = [report(release, fixture=bool(args.fixture))]
        if issues is not None:
            sections.append(issues_report(issues, fixture=bool(args.issues_file or args.fixture)))
        print("\n\n".join(sections))
        return 0
    except Exception as exc:
        print(f"OBS upstream check unverified: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
