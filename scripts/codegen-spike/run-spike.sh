#!/usr/bin/env bash
# run-spike.sh — Run the M10 code-gen spike for one or more Ollama models.
#
# For each (model, prompt) pair: generate a single-file overlay via the local
# Ollama HTTP API, validate it with scripts/validate-overlay.py, and store the
# artifact + metadata under results/<model>/<prompt-name>.html + .meta.json.
#
# Usage:
#   scripts/codegen-spike/run-spike.sh                          # all prompts, default models
#   scripts/codegen-spike/run-spike.sh qwen2.5-coder:7b         # one model
#   scripts/codegen-spike/run-spike.sh qwen2.5-coder:7b devstral:latest
#
# Environment:
#   OLLAMA_URL   (default http://localhost:11434)
#   NUM_CTX      (default 8192)  context window for generation
#   TIMEOUT_SECS (default 300)   per-request timeout

set -uo pipefail
cd "$(dirname "$0")"

OLLAMA_URL="${OLLAMA_URL:-http://localhost:11434}"
NUM_CTX="${NUM_CTX:-8192}"
TIMEOUT_SECS="${TIMEOUT_SECS:-300}"
REPO_ROOT="$(cd ../.. && pwd)"
VALIDATOR="$REPO_ROOT/scripts/validate-overlay.py"

OUTPUT_CONTRACT='Output contract (strictly follow):
- Reply with ONLY a JSON object: {"file": "<complete single-file HTML>"}
- The HTML must be fully self-contained: no external URLs, no CDN, no imports.
- Single <!DOCTYPE html> file with inline <style> and <script>.
- The page background must be transparent-friendly (no solid body background).
- No scrollbars, no document.write.'

failed=0
for model in "$@"; do
  slug="${model//[:\/]/_}"
  mkdir -p "results/$slug"
  echo "=== model: $model ==="
  for prompt_file in prompts/*.prompt.md; do
    name="$(basename "$prompt_file" .prompt.md)"
    out="results/$slug/$name.html"
    meta="results/$slug/$name.meta.json"

    payload=$(MODEL="$model" NUM_CTX="$NUM_CTX" PROMPT_FILE="$prompt_file" \
      OUTPUT_CONTRACT="$OUTPUT_CONTRACT" python build-payload.py)

    echo "--- $name"
    start=$(date +%s)
    resp=$(curl -s --max-time "$TIMEOUT_SECS" -X POST "$OLLAMA_URL/api/generate" -d "$payload")
    end=$(date +%s)
    dur=$((end - start))

    if [[ -z "$resp" ]]; then
      echo "    ERROR: empty response after ${dur}s"
      printf '{"model": "%s", "prompt": "%s", "error": "empty response", "duration_secs": %d}\n' \
        "$model" "$name" "$dur" > "$meta"
      failed=1
      continue
    fi

    # Extract HTML + metrics from the response; writes $out and prints meta JSON.
    if RESP="$resp" OUT="$out" META="$meta" NAME="$name" MODEL="$model" \
         python extract-response.py; then
      if val_out=$(python "$VALIDATOR" "$out" 2>&1); then
        echo "    generated in ${dur}s, validation: PASS"
      else
        echo "    generated in ${dur}s, validation: FAIL"
        echo "$val_out" | sed 's/^/      /'
        failed=1
      fi
    else
      echo "    ERROR: no HTML extracted after ${dur}s"
      failed=1
    fi
  done
done
exit $failed
