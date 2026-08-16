#!/usr/bin/env python3
"""Verify regenerated branding assets match the committed files.

Run `scripts/generate-assets.sh` first; this script then decodes each asset to
raw RGBA pixels and compares them against `git show HEAD:<path>` decoded the
same way. Comparing *pixels* (rather than file bytes) tolerates differences in
PNG/zlib compression between environments while still catching any real
content drift: color, layout, text, dimensions.

The macOS .icns is compared by unpacking each embedded PNG and checking it the
same way.

Usage: check-assets.py [--magick BIN]

Exit code 0 only if every asset matches.
"""

import argparse
import os
import shutil
import struct
import subprocess
import sys
import tempfile

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

PNG_ASSETS = [
    "docs/thumbnail.png",
    "docs/social-preview.png",
    "docs/opengraph.png",
    "packaging/rivulet.png",
]
ICNS_ASSET = "packaging/rivulet.icns"


def die(message):
    print(f"error: {message}", file=sys.stderr)
    sys.exit(2)


def resolve_magick(explicit):
    """Return an ImageMagick binary that can decode PNGs (magick or convert)."""
    candidates = [explicit] if explicit else ["magick", "convert"]
    for binary in candidates:
        if binary and shutil.which(binary):
            return binary
    die("ImageMagick not found (need `magick` or `convert`)")


def git_show(rel):
    result = subprocess.run(
        ["git", "show", f"HEAD:{rel}"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=REPO_ROOT,
        check=False,
    )
    if result.returncode != 0:
        die(f"git show HEAD:{rel} failed: {result.stderr.decode().strip()}")
    return result.stdout


def raw_rgba(magick, path):
    """Decode an image to 8-bit RGBA pixels (deterministic, lossless)."""
    result = subprocess.run(
        [magick, path, "-depth", "8", "-alpha", "on", "RGBA:-"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=REPO_ROOT,
        check=False,
    )
    if result.returncode != 0:
        die(f"failed to decode {path}: {result.stderr.decode().strip()}")
    return result.stdout


def icns_png_chunks(data):
    if data[:4] != b"icns":
        raise ValueError("not an ICNS file")
    offset = 8
    chunks = {}
    while offset < len(data):
        type_code = data[offset : offset + 4]
        length = struct.unpack(">I", data[offset + 4 : offset + 8])[0]
        chunks[type_code] = data[offset + 8 : offset + length]
        offset += length
    return chunks


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--magick", help="ImageMagick binary to use")
    args = parser.parse_args()

    magick = resolve_magick(args.magick)
    failures = []

    with tempfile.TemporaryDirectory() as tmp:
        # Flat PNG assets.
        for rel in PNG_ASSETS:
            committed = os.path.join(tmp, os.path.basename(rel) + ".committed.png")
            with open(committed, "wb") as fh:
                fh.write(git_show(rel))
            regenerated = os.path.join(REPO_ROOT, rel)
            ok = raw_rgba(magick, committed) == raw_rgba(magick, regenerated)
            print(f"{'OK  ' if ok else 'DIFF'} {rel}")
            if not ok:
                failures.append(rel)

        # ICNS: compare each embedded PNG.
        regenerated_bytes = open(os.path.join(REPO_ROOT, ICNS_ASSET), "rb").read()
        committed_chunks = icns_png_chunks(git_show(ICNS_ASSET))
        regenerated_chunks = icns_png_chunks(regenerated_bytes)
        if set(committed_chunks) != set(regenerated_chunks):
            print(f"DIFF {ICNS_ASSET} (icon type set differs)")
            failures.append(ICNS_ASSET)
        else:
            ok = True
            for type_code in sorted(committed_chunks):
                c = os.path.join(tmp, f"icns-{type_code.decode()}-committed.png")
                r = os.path.join(tmp, f"icns-{type_code.decode()}-regenerated.png")
                with open(c, "wb") as fh:
                    fh.write(committed_chunks[type_code])
                with open(r, "wb") as fh:
                    fh.write(regenerated_chunks[type_code])
                if raw_rgba(magick, c) != raw_rgba(magick, r):
                    print(f"DIFF {ICNS_ASSET} [{type_code.decode()}]")
                    failures.append(ICNS_ASSET)
                    ok = False
                    break
            if ok:
                print(f"OK   {ICNS_ASSET} ({len(committed_chunks)} icons)")

    if failures:
        print(
            "\nAssets drifted from the generator output. Run"
            " `scripts/generate-assets.sh` and commit the result.",
            file=sys.stderr,
        )
        return 1
    print("All assets match the committed files.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
