#!/usr/bin/env python3
"""Pack PNG images into a macOS .icns icon container.

No third-party dependencies; the ICNS format is a simple header plus a
sequence of (type, length, data) chunks where each PNG is stored verbatim.

Usage:
    png2icns.py OUT.icns TYPE=PATH [TYPE=PATH ...]

TYPE is a four-character ICNS OSType (e.g. ic09, ic10). The chunks are
written in the order given, so pass them in a fixed order for byte-for-byte
reproducible output.
"""

import struct
import sys

ICNS_MAGIC = b"icns"
PNG_MAGIC = b"\x89PNG\r\n\x1a\n"


def die(message: str) -> "None":
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def read_png(path: str) -> bytes:
    try:
        with open(path, "rb") as fh:
            data = fh.read()
    except OSError as exc:
        die(f"cannot read {path}: {exc}")
    if not data.startswith(PNG_MAGIC):
        die(f"{path} is not a PNG file")
    return data


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        die("usage: png2icns.py OUT.icns TYPE=PATH [TYPE=PATH ...]")

    out_path = argv[1]
    entries = []  # list of (type_bytes, png_bytes)
    for spec in argv[2:]:
        if "=" not in spec:
            die(f"invalid entry {spec!r}, expected TYPE=PATH")
        type_code, png_path = spec.split("=", 1)
        if len(type_code) != 4:
            die(f"invalid ICNS type {type_code!r}, must be 4 characters")
        entries.append((type_code.encode("ascii"), read_png(png_path)))

    # Header (8 bytes) + one 8-byte chunk header per entry + PNG payloads.
    total = 8 + sum(8 + len(png) for _, png in entries)
    body = bytearray()
    body += ICNS_MAGIC
    body += struct.pack(">I", total)
    for type_bytes, png in entries:
        body += type_bytes
        body += struct.pack(">I", 8 + len(png))
        body += png

    if len(body) != total:
        die(f"internal error: wrote {len(body)} bytes, expected {total}")

    try:
        with open(out_path, "wb") as fh:
            fh.write(body)
    except OSError as exc:
        die(f"cannot write {out_path}: {exc}")

    print(f"Wrote {out_path} ({total} bytes, {len(entries)} icons)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
