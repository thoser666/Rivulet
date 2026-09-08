# VST3 Host Runtime Boundary (Z96-1)

This document describes the host runtime boundary for VST3 plugin hosting,
implemented as subtask Z96-1 of [issue #96](https://github.com/thoser666/Rivulet/issues/96).

## Overview

Before a real VST3 host runtime (COM on Windows, dlopen/dylib on macOS/Linux)
is integrated, the **host boundary** is modeled as a contract so the rest of
the pipeline and GUI can already target the host, and the host runtime can
later be implemented independently.

## Key Types

### `HostLoadResult`

Outcome of attempting to load a single VST3 plugin:

```rust
pub enum HostLoadResult {
    Loaded { plugin: VstPlugin, handle: HostHandle },
    Skipped { plugin: VstPlugin, reason: SkipReason },
}
```

A missing or broken bundle produces `Skipped` — the same pattern as
`SkippedFilter` — so the chain remains valid and the pipeline continues.

### `SkipReason`

Why a plugin was skipped instead of loaded:

| Variant | Description |
|---|---|
| `BundleNotFound` | The `.vst3` bundle does not exist on disk |
| `BundleInvalid` | The bundle exists but is not a valid VST3 |
| `NoFactory` | No valid VST3 factory in bundle |
| `NoProcessor` | No audio processor in factory |
| `HostError(String)` | Platform-specific error (COM failure, dlopen error) |

### `HostHandle`

Opaque handle for a loaded VST3 plugin instance. The concrete type is defined
by the platform-specific host implementation; the boundary only guarantees
`Send` so it can be moved to an audio thread.

### `VstHost` trait

The host runtime boundary contract:

```rust
pub trait VstHost {
    fn load_plugin(&self, plugin: &VstPlugin) -> HostLoadResult;
}
```

Implementations must never panic or abort — errors produce `Skipped` results.

### `ChainLoadResults`

Summary of loading all plugins in a `VstChain` through a host:

```rust
pub struct ChainLoadResults {
    pub results: Vec<HostLoadResult>,
}
```

Provides `loaded_count()`, `skipped_count()`, `all_loaded()`, and
`skipped_plugins()` for diagnostics.

### `load_chain()`

Load every plugin in a `VstChain` through a `VstHost` and collect results.
Skipped plugins are logged but never cause a failure.

## Testability

The boundary is fully testable without a real plugin binary:

- `MockSuccessHost` — always loads (happy path)
- `MockSkipHost` — always skips (missing/broken bundle)
- `MixedHost` — mixed results (partial success)

Tests cover: loaded/skipped classification, skip reason descriptions,
handle identity, empty chain, mixed results, chain validation after load.

## Skip-on-Error Pattern

The skip-on-error contract mirrors `SkippedFilter`:

1. A plugin that fails to load produces `HostLoadResult::Skipped`
2. Skipped plugins do **not** appear in the active processing chain
3. The `VstChain` remains validatable regardless of host results
4. The pipeline continues without the skipped plugin
5. No fatal errors — the user sees a warning, not a crash

## Platform Matrix

| Platform | Host Runtime | Status |
|---|---|---|
| Windows | COM (`WindowsVstHost`, Z96-2) | Skeleton shipped (load stages); real audio routing open |
| macOS | dlopen/dylib | Follow-up (stubs skip cleanly) |
| Linux | dlopen/dylib | Follow-up (stubs skip cleanly) |

Gating: the `vst3_host_boundary_and_skeleton_are_wired` pinning guard in
`rivulet-core/tests/ci_pinning.rs` pins this contract, the Windows skeleton
and the skip-path tests to source and docs — silent regressions fail CI. The
skip-on-error semantics are enforced by the Z96-3 tests: a missing or broken
bundle can never invalidate the chain or crash the pipeline.
