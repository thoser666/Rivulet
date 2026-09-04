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
    "?state=open&per_page=100&sort=updated&direction=desc"
)
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


def fetch_open_issues(repo: str) -> list[dict]:
    """Fetch the repository's open issues (issues API returns PRs too — they
    are filtered out downstream by the `pull_request` key). Uses the
    GITHUB_TOKEN when available so private/fork runs work identically.
    """
    headers = {"Accept": "application/vnd.github+json", "User-Agent": "rivulet-obs-check"}
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(ISSUES_API.format(repo=repo), headers=headers)
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


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
