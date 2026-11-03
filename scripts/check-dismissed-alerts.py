#!/usr/bin/env python3
"""Fail when a dismissed code-scanning alert's revisit condition is back.

`check-code-scanning-alerts.py` asks GitHub for *open* alerts. A dismissal is
the other half of that policy: it removes an alert from that view, so from then
on nothing notices when the reason for the dismissal stops being true. Alert
#93 (`rust/cleartext-logging`) has exactly that shape - it was dismissed as a
false positive because the chat roster cannot carry a token, and the dismissal
is only honest while that stays the whole story.

The register lives in `docs/security.md` ("Dismissed alert register"), not in
this script, so the argument stays where a reader looks for it. This script
enforces it, because a dismissal that nothing re-checks is a decision with no
expiry date:

  * a dismissed alert without a register row is a failure - a dismissal nobody
    documented cannot be audited;
  * a row whose alert is no longer dismissed (fixed, or re-opened) is a
    failure - the register has to shrink when its reason disappears;
  * a row whose documented reason disagrees with the dismissal GitHub recorded
    is a failure;
  * a row whose machine check is true again is a failure - the revisit
    condition returned, so the alert must be triaged again.

Machine checks are a deliberately small DSL, because a register nobody can
evaluate mechanically is prose with a table around it:

  derive-debug:<path>:<Type>   true when `<Type>` in `<path>` carries
                               `#[derive(.., Debug, ..)]` again.

Usage:
  scripts/check-dismissed-alerts.py [--repo OWNER/NAME] [--json | --comment]
                                    [--self-test]

Requires `gh` and a token with `security-events: read` (`GH_TOKEN`, or
`GITHUB_TOKEN` inside Actions). `--self-test` needs neither: it runs the parser
and the evaluator against inline fixtures and never touches the network.
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

# Unicode in the --comment output must survive non-UTF-8 consoles (cp1252 on
# Windows); reconfiguring is a no-op on Linux.
for _stream in (sys.stdout, sys.stderr):
    _reconfigure = getattr(_stream, "reconfigure", None)
    if _reconfigure is not None:
        try:
            _reconfigure(encoding="utf-8")
        except (ValueError, OSError):
            pass

SECURITY_DOC = "docs/security.md"
REGISTER_HEADING = "### Dismissed alert register"
REGISTER_COLUMNS = ["alert", "rule", "reason", "revisit when", "machine check"]
DERIVE_CHECK = "derive-debug"

# Dismissals that predate the register are grandfathered: they were made
# before there was a place to record a revisit condition, and demanding a row
# for each of them would fail the gate on day one for reasons nobody can still
# reconstruct. Every dismissal from this date on needs a row - the checker
# reports the grandfathered ones without failing.
REGISTER_ENFORCED_FROM = "2026-11-03"


class RegisterError(ValueError):
    """The register in docs/security.md cannot be read as written."""


def default_repo() -> str:
    """Resolve OWNER/NAME from the environment Actions provides."""
    for name in ("GITHUB_REPOSITORY", "GH_REPO"):
        value = os.environ.get(name)
        if value and "/" in value:
            return value
    return ""


def _clean(cell: str) -> str:
    return cell.strip().strip("`").strip()


def parse_alert_number(cell: str) -> int:
    """`#93` -> 93. Anything else is a malformed register row."""
    match = re.fullmatch(r"#?(\d+)", cell.strip())
    if not match:
        raise RegisterError(f"register alert cell is not '#<number>': {cell!r}" if cell else "register alert cell is empty")
    return int(match.group(1))


def parse_register(text: str) -> list[dict]:
    """Read the dismissed-alert register out of docs/security.md.

    Every row is validated here rather than at use site: a table that has been
    edited into a shape the evaluator cannot read must fail loudly, not silently
    check nothing.
    """
    lines = text.splitlines()
    start = next(
        (i for i, line in enumerate(lines) if line.strip() == REGISTER_HEADING),
        None,
    )
    if start is None:
        raise RegisterError(f"{SECURITY_DOC} must contain the '{REGISTER_HEADING}' section")

    rows: list[dict] = []
    header = None
    for line in lines[start + 1 :]:
        stripped = line.strip()
        if stripped.startswith("#"):
            break
        if not stripped.startswith("|"):
            continue
        values = [_clean(cell) for cell in stripped.strip("|").split("|")]
        if header is None:
            header = [value.lower() for value in values]
            if header != REGISTER_COLUMNS:
                raise RegisterError(
                    "register header must be: " + " | ".join(REGISTER_COLUMNS)
                )
            continue
        if all(value and set(value) <= set("-: ") for value in values):
            continue  # markdown separator row
        if len(values) != len(REGISTER_COLUMNS):
            raise RegisterError(
                f"register row has {len(values)} cells, expected {len(REGISTER_COLUMNS)}: {stripped}"
            )
        row = dict(zip(REGISTER_COLUMNS, values))
        row["alert"] = parse_alert_number(row["alert"])
        for column in ("rule", "reason", "machine check"):
            if not row[column]:
                raise RegisterError(f"register row for #{row['alert']} has no {column}")
        if not row["revisit when"]:
            raise RegisterError(
                f"register row for #{row['alert']} has no revisit condition"
            )
        rows.append(row)

    if header is None:
        raise RegisterError(
            f"'{REGISTER_HEADING}' must contain a markdown table with the columns: "
            + " | ".join(REGISTER_COLUMNS)
        )
    numbers = [row["alert"] for row in rows]
    duplicates = {number for number in numbers if numbers.count(number) > 1}
    if duplicates:
        raise RegisterError(
            "register lists these alerts more than once: "
            + ", ".join(f"#{number}" for number in sorted(duplicates))
        )
    return rows


def split_checks(cell: str) -> list[str]:
    """Split a machine-check cell into its `;`-separated checks."""
    return [part.strip() for part in cell.split(";") if part.strip()]


def parse_check(spec: str) -> tuple[str, str, str]:
    """Split a machine check into (kind, path, symbol)."""
    parts = spec.split(":")
    if len(parts) != 3 or parts[0] != DERIVE_CHECK or not parts[1] or not parts[2]:
        raise RegisterError(
            f"unknown machine check {spec!r}; the supported kind is "
            f"'{DERIVE_CHECK}:<path>:<Type>'"
        )
    return parts[0], parts[1], parts[2]


def _attribute_block_above(lines: list[str], index: int) -> str:
    """The contiguous attribute/doc block directly above `lines[index]`."""
    collected: list[str] = []
    position = index - 1
    while position >= 0:
        stripped = lines[position].strip()
        if not stripped:
            break
        if (
            stripped.startswith("///")
            or stripped.startswith("//")
            or stripped.startswith("#[")
            or stripped.startswith(")")
            or stripped.startswith("]")
            or stripped == ","
            or stripped.endswith(",")
        ):
            collected.append(lines[position])
            position -= 1
            continue
        break
    return "\n".join(reversed(collected))


def derives_debug(source: str, type_name: str) -> bool:
    """True when `pub struct <type_name>` still carries `#[derive(.., Debug)]`.

    Multi-line derive attributes are joined before the match so a reformatted
    attribute cannot slip past the check.
    """
    lines = source.splitlines()
    marker = f"pub struct {type_name}"
    for index, line in enumerate(lines):
        if not line.strip().startswith(marker):
            continue
        block = _attribute_block_above(lines, index)
        for group in re.findall(r"#\[derive\((.*?)\)\]", block, re.S):
            if re.search(r"\bDebug\b", group):
                return True
        return False
    raise RegisterError(f"struct {type_name} not found in the source")


def evaluate(
    register: list[dict],
    dismissed: dict[int, dict],
    read_source,
    enforced_from: str = REGISTER_ENFORCED_FROM,
) -> tuple[list[str], int]:
    """Cross-check the register against the live alert states and the sources.

    Returns `(failures, legacy)`; empty failures is a pass. `legacy` counts the
    dismissals that predate `enforced_from` and therefore the register.
    """
    failures: list[str] = []
    legacy = 0
    registered = {row["alert"]: row for row in register}

    for number in sorted(dismissed):
        if number in registered:
            continue
        alert = dismissed[number]
        rule = (alert.get("rule") or {}).get("id", "unknown-rule")
        stamp = str(alert.get("dismissed_at") or "")[:10]
        if not stamp:
            failures.append(
                f"#{number} [{rule}] is dismissed without a dismissal timestamp - "
                "it cannot be placed against the register"
            )
            continue
        if stamp < enforced_from:
            legacy += 1
            continue
        failures.append(
            f"#{number} [{rule}] was dismissed on {stamp} without a register row - "
            f"add one to '{REGISTER_HEADING}' in {SECURITY_DOC} or re-open it"
        )

    for number in sorted(registered):
        row = registered[number]
        alert = dismissed.get(number)
        if alert is None:
            failures.append(
                f"#{number} has a register row but is not dismissed any more - "
                "delete the row (a fixed alert needs no revisit condition)"
            )
            continue

        recorded = (alert.get("dismissed_reason") or "").strip().lower()
        documented = row["reason"].strip().lower()
        if recorded and recorded != documented:
            failures.append(
                f"#{number} is dismissed as '{recorded}' but the register documents "
                f"'{documented}'"
            )

        for spec in split_checks(row["machine check"]):
            try:
                kind, path, type_name = parse_check(spec)
            except RegisterError as error:
                failures.append(f"#{number}: {error}")
                continue
            if kind != DERIVE_CHECK:
                failures.append(f"#{number}: unsupported machine check kind '{kind}'")
                continue

            source = read_source(path)
            if source is None:
                failures.append(
                    f"#{number} machine check points at a file that does not exist: {path}"
                )
                continue
            try:
                back = derives_debug(source, type_name)
            except RegisterError as error:
                failures.append(f"#{number}: {error}")
                continue
            if back:
                failures.append(
                    f"#{number} revisit condition is back: `{type_name}` in {path} derives "
                    "`Debug` again - re-triage the alert before dismissing it again"
                )

    return failures, legacy


def fetch_dismissed_alerts(repo: str) -> dict[int, dict]:
    """Read every dismissed code-scanning alert via the GitHub API."""
    endpoint = f"repos/{repo}/code-scanning/alerts"
    proc = subprocess.run(
        [
            "gh",
            "api",
            "-H",
            "Accept: application/vnd.github+json",
            "--paginate",
            f"{endpoint}?state=dismissed&per_page=100",
        ],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"gh api failed ({proc.returncode}): {proc.stderr.strip() or 'no stderr'}"
        )
    raw = proc.stdout.strip()
    if not raw:
        return {}
    decoder = json.JSONDecoder()
    alerts: dict[int, dict] = {}
    index = 0
    while index < len(raw):
        while index < len(raw) and raw[index].isspace():
            index += 1
        if index >= len(raw):
            break
        chunk, index = decoder.raw_decode(raw, index)
        if isinstance(chunk, dict):
            chunk = [chunk]
        for item in chunk:
            # `gh api --paginate` emits one JSON array per page (`[...][...]`),
            # while a slurped or nested response arrives as arrays of arrays.
            # Both shapes must be read: a silently empty result would make every
            # dismissal look registered and turn this check into a no-op.
            if isinstance(item, list):
                for alert in item:
                    if isinstance(alert, dict) and isinstance(alert.get("number"), int):
                        alerts[alert["number"]] = alert
            elif isinstance(item, dict) and isinstance(item.get("number"), int):
                alerts[item["number"]] = item
    return alerts


def render_comment(failures: list[str], checked: int, legacy: int = 0) -> str:
    """Markdown block for the Actions step summary."""
    lines = ["## Dismissed code-scanning alerts", ""]
    if failures:
        lines.append(f"**{len(failures)} dismissal(s) need attention:**")
        lines.append("")
        lines += [f"- {failure}" for failure in failures]
    else:
        lines.append(
            f"All {checked - legacy} registered dismissal(s) are still dismissed for the "
            "documented reason. ✅"
        )
    if legacy:
        lines.append("")
        lines.append(
            f"_{legacy} dismissal(s) predate the register (before "
            f"{REGISTER_ENFORCED_FROM}) and carry no revisit condition._"
        )
    return "\n".join(lines)


def self_test() -> None:
    """Guard the register parser and the evaluator against silent regressions."""
    doc = "\n".join(
        [
            "# Security",
            "",
            REGISTER_HEADING,
            "",
            "| Alert | Rule | Reason | Revisit when | Machine check |",
            "| --- | --- | --- | --- | --- |",
            "| #93 | `rust/cleartext-logging` | `false positive` | a chat config derives `Debug` again | `derive-debug:rivulet-core/src/chat.rs:ChatConfig` |",
            "| #7 | `RuleID` | `used in tests` | the fixture is removed | `derive-debug:src/x.rs:Thing` |",
            "",
            "### Dependabot alert management",
            "",
            "| Alert | ignored |",
        ]
    )
    rows = parse_register(doc)
    assert [row["alert"] for row in rows] == [93, 7], rows
    assert rows[0]["reason"] == "false positive", rows[0]
    assert rows[0]["machine check"] == "derive-debug:rivulet-core/src/chat.rs:ChatConfig"
    # The register stops at the next heading: an unrelated table below must not
    # be parsed as register rows.
    assert len(rows) == 2

    # A missing section, a wrong header and a malformed row all fail loudly.
    for broken in (
        "# Security\n\nno register here\n",
        doc.replace("| Reason |", "| Why |"),
        doc.replace("| #93 |", "| ninety-three |"),
        doc.replace("| a chat config derives `Debug` again |", "|  |"),
    ):
        try:
            parse_register(broken)
        except RegisterError:
            pass
        else:  # pragma: no cover - the assertion below is the point
            raise AssertionError(f"parse_register accepted a broken register: {broken!r}")

    source_derived = '/// doc\n#[derive(Debug, Clone)]\npub struct ChatConfig {\n}\n'
    source_manual = "/// doc\n#[derive(Clone)]\npub struct ChatConfig {\n}\n\nimpl std::fmt::Debug for ChatConfig {}\n"
    source_multiline = "#[derive(\n    Clone,\n    Debug,\n)]\npub struct ChatConfig {}\n"
    assert derives_debug(source_derived, "ChatConfig")
    assert not derives_debug(source_manual, "ChatConfig")
    assert derives_debug(source_multiline, "ChatConfig")
    # A derive that belongs to the *previous* item must not be attributed to
    # this struct.
    previous_item = "#[derive(Debug)]\npub struct Other;\n\n#[derive(Clone)]\npub struct ChatConfig {}\n"
    assert not derives_debug(previous_item, "ChatConfig")
    try:
        derives_debug("pub struct Something;", "ChatConfig")
    except RegisterError:
        pass
    else:  # pragma: no cover
        raise AssertionError("a missing struct must be an error, not a silent pass")

    dismissed = {
        93: {
            "number": 93,
            "rule": {"id": "rust/cleartext-logging"},
            "dismissed_reason": "false positive",
            "dismissed_at": "2026-11-05T12:00:00Z",
        },
        7: {
            "number": 7,
            "rule": {"id": "RuleID"},
            "dismissed_reason": "used in tests",
            "dismissed_at": "2025-06-01T12:00:00Z",
        },
    }
    source_other = "#[derive(Clone)]" + chr(10) + "pub struct Thing {}" + chr(10)
    sources = {"rivulet-core/src/chat.rs": source_manual, "src/x.rs": source_other}
    assert evaluate(rows, dismissed, sources.get)[0] == []

    # The revisit condition coming back is the failure this script exists for.
    back, _ = evaluate(rows, dismissed, {**sources, "rivulet-core/src/chat.rs": source_derived}.get)
    assert len(back) == 1 and "#93 revisit condition is back" in back[0], back

    # A dismissal nobody registered fails, and so does a stale row.
    unregistered, _ = evaluate(
        rows,
        {
            **dismissed,
            12: {
                "number": 12,
                "rule": {"id": "R"},
                "dismissed_at": "2026-11-04T09:00:00Z",
            },
        },
        sources.get,
    )
    assert any("#12" in failure and "without a register row" in failure for failure in unregistered), unregistered
    stale, _ = evaluate(rows, {93: dismissed[93]}, sources.get)
    assert any("#7 has a register row but is not dismissed" in failure for failure in stale), stale

    # A documented reason that disagrees with GitHub's record fails.
    drifted, _ = evaluate(
        rows,
        {**dismissed, 93: {**dismissed[93], "dismissed_reason": "won't fix"}},
        sources.get,
    )
    assert any("#93 is dismissed as 'won't fix'" in failure for failure in drifted), drifted

    # Dismissals older than the register are counted, not failed: they predate
    # the place where a revisit condition could have been written down.
    old_failures, legacy = evaluate(
        rows,
        {
            93: dismissed[93],
            7: dismissed[7],
            5: {
                "number": 5,
                "rule": {"id": "Old"},
                "dismissed_at": "2024-01-01T00:00:00Z",
            },
        },
        sources.get,
    )
    assert old_failures == [] and legacy == 1, (old_failures, legacy)
    # A dismissal with no timestamp cannot be placed and is a failure.
    no_stamp, _ = evaluate(rows, {93: dismissed[93], 7: dismissed[7], 4: {"number": 4, "rule": {"id": "R"}}}, sources.get)
    assert any("#4" in failure and "timestamp" in failure for failure in no_stamp), no_stamp

    # A cell may carry several checks separated by ';'.
    multi = [
        {
            **rows[0],
            "machine check": (
                "derive-debug:rivulet-core/src/chat.rs:ChatConfig;"
                " derive-debug:src/x.rs:Thing"
            ),
        }
    ]
    assert evaluate(multi, {93: dismissed[93]}, sources.get)[0] == []
    multi_back = [
        {**multi[0], "machine check": multi[0]["machine check"].replace("src/x.rs:Thing", "src/x.rs:ChatConfig")}
    ]
    assert any(
        "revisit condition is back" in failure
        for failure in evaluate(multi_back, {93: dismissed[93]}, {**sources, "src/x.rs": source_derived}.get)[0]
    )

    # Unknown checks and missing files are failures, never silent passes.
    bad_check = [{**rows[0], "machine check": "grep-for:whatever"}]
    assert any("unknown machine check" in failure for failure in evaluate(bad_check, dismissed, sources.get)[0])
    missing_file, _ = evaluate(rows, dismissed, lambda path: None)
    assert any("does not exist" in failure for failure in missing_file), missing_file

    print("check-dismissed-alerts: self-test passed")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", default=default_repo(), help="OWNER/NAME (default: $GITHUB_REPOSITORY)")
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    parser.add_argument("--comment", action="store_true", help="Markdown output for the step summary")
    parser.add_argument("--self-test", action="store_true", help="run the built-in tests and exit")
    args = parser.parse_args(argv)

    if args.self_test:
        self_test()
        return 0

    try:
        register = parse_register(Path(SECURITY_DOC).read_text(encoding="utf-8"))
    except (OSError, RegisterError) as error:
        print(f"check-dismissed-alerts: {error}", file=sys.stderr)
        return 2

    if not args.repo:
        print(
            "check-dismissed-alerts: no repository - pass --repo or set GITHUB_REPOSITORY",
            file=sys.stderr,
        )
        return 2

    try:
        dismissed = fetch_dismissed_alerts(args.repo)
    except (RuntimeError, OSError) as error:
        print(f"check-dismissed-alerts: {error}", file=sys.stderr)
        return 2

    def read_source(path: str):
        try:
            return Path(path).read_text(encoding="utf-8")
        except OSError:
            return None

    failures, legacy = evaluate(register, dismissed, read_source)

    if args.json:
        print(json.dumps(
            {
                "repository": args.repo,
                "registered": [row["alert"] for row in register],
                "dismissed": sorted(dismissed),
                "grandfathered": legacy,
                "failures": failures,
            },
            indent=2,
        ))
    elif args.comment:
        print(render_comment(failures, len(dismissed), legacy))
    else:
        for failure in failures:
            print(f"dismissed alert needs attention: {failure}")
        if not failures:
            print(
                f"check-dismissed-alerts: {len(dismissed)} dismissed alert(s) in "
                f"{args.repo} match the register"
            )

    if failures:
        print(
            "check-dismissed-alerts: a dismissal is only valid while its documented "
            f"revisit condition is false; see {SECURITY_DOC} "
            f"('{REGISTER_HEADING}').",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    sys.exit(main())
