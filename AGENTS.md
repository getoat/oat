# Oat Agent Guidelines

This file applies to everything under `/root/oat`.

## Workflow

- Keep `Cargo.lock` committed for this application crate.
- Prefer focused, local changes over broad refactors unless the task requires it.
- TUI performance matters. After adding or changing TUI features, do a performance pass and remove any obvious hotspots before finishing.
- Before finishing Rust changes, run `cargo fmt --check` and `cargo test`.

## Testing

- Add unit tests where behavior can be exercised directly, especially for state transitions, input handling, parsing/formatting helpers, and regression-prone edge cases.
- Keep tests proportional to the logic. Thin glue code and brittle terminal rendering details do not need exhaustive unit coverage unless they hide meaningful behavior.
- Prefer colocated `#[cfg(test)]` modules near the code they validate.
- When fixing a bug, add or update a test when practical so the failure mode stays covered.

## Git operations

- Always use scoped conventional commit messages, as this is required for the automated versioning system to work properly.
