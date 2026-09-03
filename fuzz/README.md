# Fuzz targets

libFuzzer targets for every parser that consumes remote-controlled input.
Run with [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) on the
nightly toolchain:

```bash
cargo install cargo-fuzz --locked
rustup toolchain install nightly --profile minimal
bash scripts/fuzz-smoke.sh            # short regression run (as in CI)
```

| Target | Crate | Entry point | Untrusted source |
|---|---|---|---|
| `parse_irc_line` | rivulet-core | `twitch_chat::parse_irc_line` | Twitch IRC socket lines |
| `sdp_offer_endpoint` | rivulet-core | `sdp::SdpOffer::h264_opus` | WHIP endpoint string (Settings/presets) |
| `parse_latest_release` | rivulet-updater | `updater::parse_latest_release` | GitHub Releases API JSON |
| `parse_checksums` | rivulet-updater | `updater::parse_checksums` | `SHA256SUMS` manifest from the release page |

Targets assert invariants beyond "no panic" (e.g. no empty users, no NUL
bytes, structural SDP properties) — if a target fails, a real parser
regression was found. The corpus lives in `fuzz/corpus/`, crash inputs in
`fuzz/artifacts/` (both gitignored); CI uploads crash artifacts on failure.

### Deep campaign (scheduled weekly)

Beyond the push-time smoke, the **Deep fuzz (weekly)** workflow
(`.github/workflows/fuzz-deep.yml`) runs every Monday and gives each target
a 10-minute budget:

```bash
FUZZ_MAX_TOTAL_TIME=600 bash scripts/fuzz-smoke.sh   # what CI runs
# or manually, per target:
cd fuzz && cargo +nightly fuzz run parse_irc_line -- -max_total_time=600
```

The corpus is persisted between runs through the actions cache
(`fuzz/corpus/`, restored via `restore-keys: fuzz-corpus-` and saved under
a per-run key), so coverage accumulates across weeks instead of restarting
from zero; GitHub evicts old cache entries by LRU. A crash uploads the
crashing input as `fuzz-deep-crashes` artifact. The workflow can also be
triggered manually (`workflow_dispatch`) before dependency upgrades that
touch a parser's dependencies.

Windows note: the ASan runtime comes from the Visual Studio
"C++ AddressSanitizer" component, which rustup does not ship; use WSL or
the Linux CI gate there.
