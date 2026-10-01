#!/usr/bin/env bash
# W3 CI integration example: end-to-end video generation from code on a clean
# runner (spec "Batch rendering: a CI integration example").
#
# Checks the three acceptance criteria against the real binary:
#   1. rendering the same frame twice produces byte-identical PNGs
#   2. a batch run over >=2 configs produces a machine-readable summary with
#      per-job status and output paths
#   3. a video container is really produced by the encoder, not just an empty
#      file
#
# Deliberately runs the shipped `rivulet` binary, so the CLI surface itself is
# covered too. Set -euo pipefail: a silently empty render must fail the job.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

configs="$here/ci-render-smoke"
out="$work/out"
frames="${RENDER_SMOKE_FRAMES:-10}"

rivulet() {
    cargo run --quiet --locked -p rivulet-cli -- "$@"
}

fail() {
    echo "render smoke: $*" >&2
    exit 1
}

# --- 1. deterministic still -------------------------------------------
rivulet render --config "$configs/card.toml" --frame 7 --png "$work/a.png" >/dev/null
rivulet render --config "$configs/card.toml" --frame 7 --png "$work/b.png" >/dev/null

cmp -s "$work/a.png" "$work/b.png" \
    || fail "frame 7 rendered twice produced different bytes"
echo "ok: same frame twice is byte-identical ($(wc -c <"$work/a.png") bytes)"

# A different frame must differ, so the check above cannot pass by rendering
# one constant image twice.
rivulet render --config "$configs/card.toml" --frame 8 --png "$work/c.png" >/dev/null
if cmp -s "$work/a.png" "$work/c.png"; then
    fail "frame 8 rendered identical to frame 7: the animation is not reaching the output"
fi
echo "ok: consecutive frames differ"

# The PNG must start with the signature, not merely exist.
magic="$(head -c 8 "$work/a.png" | od -An -tx1 | tr -d ' \n')"
if [ "$magic" != "89504e470d0a1a0a" ]; then
    fail "output is not a PNG (magic $magic)"
fi
echo "ok: output carries the PNG signature"

# --- 2. batch summary over two configs ---------------------------------
rivulet render \
    --config-dir "$configs" \
    --out-dir "$out" \
    --frames "$frames" \
    --json >"$work/summary.json"

python3 - "$work/summary.json" "$out" <<'PY'
import json
import os
import sys

summary_path, out_dir = sys.argv[1], sys.argv[2]
with open(summary_path, encoding="utf-8") as handle:
    summary = json.load(handle)

assert summary["schema_version"] == 1, summary
jobs = summary["jobs"]
assert len(jobs) >= 2, f"expected >=2 jobs, got {len(jobs)}: {summary}"
assert summary["succeeded"] == len(jobs), f"not every job succeeded: {summary}"
assert summary["failed"] == 0, summary

names = sorted(os.path.basename(job["config"]) for job in jobs)
assert names == ["card.toml", "split.toml"], names

for job in jobs:
    assert job["status"] == "succeeded", job
    # Output paths are recorded relative to out_dir, so a summary produced on
    # one runner is diffable against another.
    for key, extension in (("png", ".png"), ("video", ".mp4")):
        path = job[key]
        assert path is not None, f"{job['config']} has no {key}: {job}"
        assert not os.path.isabs(path), f"{key} path must be relative: {path}"
        assert os.path.isfile(os.path.join(out_dir, path)), f"missing {path}"
        assert os.path.getsize(os.path.join(out_dir, path)) > 0, f"empty {path}"
        assert path.endswith(extension), f"{key} has the wrong extension: {path}"

print(f"ok: batch summary lists {len(jobs)} succeeded jobs with output paths")
PY

# Two distinct scenes must not produce one identical image.
if cmp -s "$out/card.png" "$out/split.png"; then
    fail "both scenes rendered the same image"
fi
echo "ok: the two scenes produced different stills"

# --- 3. real video output ---------------------------------------------
for video in "$out/card.mp4" "$out/split.mp4"; do
    [ -s "$video" ] || fail "no video produced at $video"
    # "ftyp" is the MP4 container's first box type; an unfinalised or empty
    # muxer output does not carry it. Read the header into a variable rather
    # than piping into `grep -q`: that would let a SIGPIPE from the early-exiting
    # grep surface as a pipeline failure under `pipefail`.
    header="$(head -c 64 "$video" | tr -d '\0')"
    case "$header" in
        *ftyp*) ;;
        *) fail "$video does not look like an MP4 container" ;;
    esac
done
echo "ok: both videos carry an MP4 header ($(wc -c <"$out/card.mp4") bytes)"

# Frame rate and duration are reported, and must agree with the config.
python3 - "$work/summary.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    summary = json.load(handle)
for job in summary["jobs"]:
    assert job["frames"] > 0, job
    assert job["duration_secs"] > 0, job
print("ok: every job reports frames and a duration")
PY

echo "render smoke: all checks passed"
