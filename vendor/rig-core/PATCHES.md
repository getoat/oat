Upstream base: `rig-core` 0.33.0 from crates.io.

Local change:
- `src/agent/prompt_request/streaming.rs`
  - fire `on_stream_completion_response_finish(...)` for every completed streamed
    substep, including tool-only substeps, so Oat can persist per-step usage
    before a later multi-turn failure.

Intentionally omitted from this vendored copy:
- `examples/`
- `tests/`
- crate packaging artifacts and release metadata not required to build Oat

If this dependency is updated, reapply the hook behavior above and rerun:
- `cargo fmt --check`
- `cargo test`
