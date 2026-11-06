#!/usr/bin/env python3
"""Resolve the container image that regenerates and checks the branding assets.

The canonical asset output depends on the pinned ImageMagick/FreeType build, not
on the Debian mirror the tool layer is installed from. Issue #391: building that
layer from ``snapshot.debian.org`` inside the required ``Lints`` job turned an
unrelated CDN outage (503/504) into a red required check on every pull request,
and the asset comparison never ran.

Resolution order:

1. ``scripts/assets-tool-image.txt`` — when it names a reference pinned by
   digest, pull it. ``.github/workflows/assets-tool-image.yml`` publishes that
   image to GHCR once, so a pull request needs no apt traffic at all.
2. ``scripts/assets-tool.Dockerfile`` — build it locally when nothing is pinned
   or the pull fails. The build retries with real backoff (30s, 120s, 300s) and
   its apt layer falls back from the Debian snapshot to ``deb.debian.org``, so a
   mirror outage cannot fail the step before the comparison runs.

Usage:
    scripts/assets-tool-image.py --resolve     # print the image reference to run
    scripts/assets-tool-image.py --build       # build the local fallback image
    scripts/assets-tool-image.py --self-test
"""

import argparse
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DOCKERFILE = REPO_ROOT / "scripts" / "assets-tool.Dockerfile"
PIN_FILE = REPO_ROOT / "scripts" / "assets-tool-image.txt"
LOCAL_TAG = "rivulet-assets-check"
WORKFLOW = ".github/workflows/assets-tool-image.yml"
MIRROR = "snapshot.debian.org"

# The first attempt runs immediately; every further attempt waits its delay.
RETRY_DELAYS = (30, 120, 300)
ATTEMPTS = len(RETRY_DELAYS) + 1


def log(message):
    print(message, file=sys.stderr)


def pinned_reference(text):
    """The first reference line of the pin file, or None when nothing is pinned."""
    for line in text.splitlines():
        stripped = line.strip()
        if stripped and not stripped.startswith("#"):
            return stripped
    return None


def reference_problem(reference):
    """Why ``reference`` may not be used, or None when it is fine."""
    if "@sha256:" not in reference:
        return (
            f"{PIN_FILE.relative_to(REPO_ROOT)} must pin the image by digest "
            f"(@sha256:...), not by tag: {reference}"
        )
    return None


def failure_message():
    delays = ", ".join(f"{delay}s" for delay in RETRY_DELAYS)
    return (
        f"assets tool image unavailable: neither the pinned pull nor {ATTEMPTS} "
        f"build attempts produced an image, so {MIRROR} and its deb.debian.org "
        f"fallback were both unusable (backoff: {delays}). Publish the image once "
        f"via {WORKFLOW} and pin the digest it prints in "
        f"{PIN_FILE.relative_to(REPO_ROOT)}, or re-run once the mirror recovers."
    )


def build(run, log_fn, sleep_fn):
    """Build the local fallback image, retrying with the documented backoff."""
    for attempt in range(1, ATTEMPTS + 1):
        built = run(
            [
                "build",
                "--network",
                "host",
                "-f",
                str(DOCKERFILE),
                "-t",
                LOCAL_TAG,
                str(REPO_ROOT),
            ]
        )
        if built == 0:
            log_fn(f"assets tool image: built {LOCAL_TAG} (attempt {attempt}/{ATTEMPTS})")
            return LOCAL_TAG
        if attempt <= len(RETRY_DELAYS):
            delay = RETRY_DELAYS[attempt - 1]
            log_fn(
                f"assets tool image: build failed (attempt {attempt}/{ATTEMPTS}); "
                f"retrying in {delay}s"
            )
            sleep_fn(delay)
    raise SystemExit(failure_message())


def resolve(text, run, log_fn, sleep_fn):
    """The image reference to run: the pinned image when available, else a build."""
    reference = pinned_reference(text)
    if reference is not None:
        problem = reference_problem(reference)
        if problem is not None:
            raise SystemExit(problem)
        if run(["pull", reference]) == 0:
            log_fn(f"assets tool image: pulled pinned {reference}")
            return reference
        log_fn(f"assets tool image: could not pull {reference}; building locally")
    return build(run, log_fn, sleep_fn)


def docker(args):
    # Docker prints its pull/build progress on stdout, and `--resolve` is read
    # with `$(...)` in ci.yml: stdout must carry the image reference and nothing
    # else, or docker run fails with "invalid reference format".
    return subprocess.run(
        ["docker", *args], check=False, stdout=sys.stderr, stderr=sys.stderr
    ).returncode


def self_test():
    slept = []
    lines = []
    messages = []

    def note(message):
        messages.append(message)

    def record(args):
        lines.append("docker " + " ".join(args))

    def succeeds(args):
        record(args)
        return 0

    def sequence(*codes):
        pending = list(codes)

        def run(args):
            record(args)
            return pending.pop(0) if pending else 1

        return run

    # The pin file ignores comments and blank lines, and an empty file means
    # "nothing pinned".
    assert pinned_reference("# note\n\nghcr.io/o/r@sha256:abc\n") == "ghcr.io/o/r@sha256:abc"
    assert pinned_reference("# only a comment\n") is None

    # A tag is not a pin.
    assert reference_problem("ghcr.io/o/r:v1") is not None
    assert reference_problem("ghcr.io/o/r@sha256:abc") is None

    # A pinned pull needs no build at all (option A: no apt traffic at PR time).
    lines.clear()
    slept.clear()
    pinned = "ghcr.io/o/r@sha256:abc"
    assert resolve(pinned, succeeds, note, slept.append) == pinned
    assert not any("build" in line for line in lines), lines

    # A failed pull falls back to exactly one build.
    lines.clear()
    slept.clear()
    assert resolve(pinned, sequence(1, 0), note, slept.append) == LOCAL_TAG
    assert slept == [], slept

    # The build retries with the documented backoff before succeeding.
    lines.clear()
    slept.clear()
    assert resolve("", sequence(1, 1, 0), note, slept.append) == LOCAL_TAG
    assert slept == [30, 120], slept
    assert any("retrying in 30s" in message for message in messages), messages

    # Exhausting every attempt names the mirror, the workflow and the budget.
    lines.clear()
    slept.clear()
    try:
        resolve("", sequence(1, 1, 1, 1), note, slept.append)
    except SystemExit as exc:
        message = str(exc)
    else:  # pragma: no cover - the test must fail loudly
        raise AssertionError("the resolver must fail when no image is available")
    assert slept == [30, 120, 300], slept
    for needle in (MIRROR, WORKFLOW, "30s, 120s, 300s", "4 build attempts"):
        assert needle in message, (needle, message)

    print("PASS [assets tool image: pin, pull, build fallback and backoff]")
    print("All self-test cases passed.")


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--resolve", action="store_true", help="print the image to run")
    group.add_argument("--build", action="store_true", help="build the fallback image")
    group.add_argument("--self-test", action="store_true", help="run the built-in tests")
    args = parser.parse_args(argv)

    if args.self_test:
        self_test()
        return 0
    if not DOCKERFILE.is_file():
        raise SystemExit(f"missing {DOCKERFILE.relative_to(REPO_ROOT)}")
    text = PIN_FILE.read_text(encoding="utf-8") if PIN_FILE.is_file() else ""
    if args.build:
        print(build(docker, log, time.sleep))
    else:
        print(resolve(text, docker, log, time.sleep))
    return 0


if __name__ == "__main__":
    sys.exit(main())
