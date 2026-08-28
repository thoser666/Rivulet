#!/usr/bin/env python3
"""Static regression checks for the reproducible RIST smoke setup."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
script = (ROOT / "scripts/rist-receiver-smoke.sh").read_text(encoding="utf-8")

assert "docker build --pull=true -t rivulet-rist-smoke:ci docker/rist-smoke" in workflow
assert "name: Inspect RIST plugin" in workflow
assert workflow.index("Build RIST smoke image for diagnostics") < workflow.index("Inspect RIST plugin")
assert "ristsrc address=0.0.0.0 port=10080" in script
assert "timeout --signal=TERM --kill-after=5s 30s" in script
assert "receiver did not remain running" in script
assert "ristsink address=\"$RECEIVER\" port=10080" in script
assert "rtpmp2tpay" in script
assert "application/x-rtp" not in script
print("RIST pipeline contract OK")
