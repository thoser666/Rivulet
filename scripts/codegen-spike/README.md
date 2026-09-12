# M10 code-gen spike — qwen2.5-coder:7b vs devstral

Methodology, prompts, and results for the code-gen model decision documented in
[`docs/m10-ai-creative-studio.md`](../../docs/m10-ai-creative-studio.md)
(Open question 1, status-tracker row "Spike").

## What we measure

Five real overlay prompts (the exact widget classes the Creative Studio ships
in phase 1) are sent to each local model via Ollama's `/api/generate` with a
strict JSON output contract. Each generated single-file overlay is scored:

| Column | Source | Automated? |
| --- | --- | --- |
| **Valid** | `scripts/validate-overlay.py` (structure, no-remote/no-CDN, no-crash, size, JS syntax via `node --check`, animation present) | ✅ |
| **API** | the required `window.rivulet*` host function exists | ✅ grep in validator-adjacent check |
| **Gen time** | wall clock per prompt (runner meta) | ✅ |
| **Visual pass** | headless-Edge screenshot (`render-screenshots.sh`, WebView2 engine) reviewed against the prompt's style/behavior asks | 👁 manual |

## Layout

- `prompts/*.prompt.md` — the five overlay prompts (follower alert, goal bar,
  chat box, poll widget, emote rain).
- `run-spike.sh <model>…` — generates + validates all prompts for each model.
- `render-screenshots.sh [results-dir]` — screenshots every artifact.
- `results/<model>/<prompt>.html` + `.meta.json` — committed artifacts.
- `results/**` PNGs are gitignored (binary, regenerable).

## Candidates

| Model | Footprint | Hardware fit (RTX 4060 Ti 8 GB) |
| --- | --- | --- |
| `qwen2.5-coder:7b` | ~4.7 GB | fully in VRAM |
| `devstral:latest` | ~14 GB | CPU/GPU split — measures the "16 GB GPU" path |
| `qwen3:8b` | ~5.2 GB | baseline (already present; not a coder model) |

> Note: the M10 doc originally named `qwen3-coder:8b`, which does not exist in
> the Ollama library (qwen3-coder ships 30b/480b only). The 8 GB-class
> candidate is `qwen2.5-coder:7b`; the doc was updated accordingly.

## Rerun

```bash
ollama pull qwen2.5-coder:7b
scripts/codegen-spike/run-spike.sh qwen2.5-coder:7b
scripts/codegen-spike/render-screenshots.sh
```
