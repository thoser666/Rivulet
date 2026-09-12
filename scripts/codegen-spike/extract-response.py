#!/usr/bin/env python3
"""Extract the generated overlay HTML from an Ollama /api/generate reply.

Reads environment: RESP (raw response JSON), OUT (artifact path),
META (meta json path), NAME, MODEL.

The model is asked (via "format": "json") to reply {"file": "<html>"}; this
helper tolerates models that wrap or paraphrase that contract.
Writes the HTML to OUT and the metrics to META. Exits 0 iff HTML was extracted.
"""

import json
import os
import sys


def extract_html(text: str) -> str | None:
    # 1) Strict: the reply itself is the contract JSON.
    try:
        inner = json.loads(text)
        html = inner.get("file")
        if html and "<" in html:
            return html
    except (json.JSONDecodeError, AttributeError):
        pass
    # 2) Reply has prose around a JSON object with "file".
    idx = text.find('{"file"')
    if idx >= 0:
        try:
            inner = json.loads(text[idx:])
            html = inner.get("file")
            if html and "<" in html:
                return html
        except json.JSONDecodeError:
            pass
    # 3) Fall back: first <!DOCTYPE html ... </html> block in the raw text.
    lower = text.lower()
    start = lower.find("<!doctype html")
    if start >= 0:
        end = lower.rfind("</html>")
        if end > start:
            return text[start : end + len("</html>")]
    return None


def main() -> int:
    resp = json.loads(os.environ["RESP"])
    html = extract_html(resp.get("response", ""))
    if not html:
        return 1
    with open(os.environ["OUT"], "w", encoding="utf-8", newline="\n") as f:
        f.write(html)
    meta = {
        "model": os.environ.get("MODEL"),
        "prompt": os.environ.get("NAME"),
        "total_duration_ns": resp.get("total_duration"),
        "load_duration_ns": resp.get("load_duration"),
        "prompt_eval_count": resp.get("prompt_eval_count"),
        "eval_count": resp.get("eval_count"),
        "eval_duration_ns": resp.get("eval_duration"),
    }
    with open(os.environ["META"], "w", encoding="utf-8", newline="\n") as f:
        json.dump(meta, f, indent=2)
        f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
