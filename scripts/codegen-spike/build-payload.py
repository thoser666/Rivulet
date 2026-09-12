#!/usr/bin/env python3
"""Build the Ollama /api/generate request body for one spike prompt.

Reads environment: MODEL, NUM_CTX, PROMPT_FILE, OUTPUT_CONTRACT.
Prints the JSON request body on stdout.
"""

import json
import os
import sys

prompt_file = os.environ["PROMPT_FILE"]
with open(prompt_file, encoding="utf-8") as f:
    prompt_text = f.read().strip()

contract = os.environ.get("OUTPUT_CONTRACT", "")
body = {
    "model": os.environ["MODEL"],
    "prompt": f"{prompt_text}\n\n{contract}",
    "stream": False,
    "format": "json",
    "options": {
        "temperature": 0.2,
        "top_p": 0.9,
        "num_ctx": int(os.environ.get("NUM_CTX", "8192")),
        "num_predict": 8192,
    },
}
print(json.dumps(body))
sys.exit(0)
