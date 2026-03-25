# Oat Architecture Refactor

## Summary

Execute one deliberate internal reshape that preserves the CLI contract and current user-facing behavior, while replacing the current monolithic ownership model with a strict layered structure.

### Non-negotiable compatibility

Preserve:

- CLI flags and behavior:
  - `oat`
  - `oat --dangerous`
  - `oat --headless "prompt"`
  - `oat --headless --dangerous "prompt"`
- current config file format
- current tool names and approval semantics
- planning wrapper tags:
  - `<planning_ready>`
  - `<proposed_plan>`
- current transcript semantics unless cleanup is required to remove accidental coupling

Internal/public Rust API compatibility is not a goal.

### Refactor objective

Optimize for:

- smaller, bounded modules
- pure domain logic separated from UI/runtime/provider code
- explicit dependency direction
- localized feature ownership
- easy reasoning about state transitions and workflows
- future growth without reintroducing monoliths

### Explicit anti-goals

Do not introduce:

- a plugin framework
- a generic event bus
- unnecessary trait abstraction
- service-locator style wiring
- “clean architecture” layers that exist only on paper

Use plain structs, enums, modules, and direct ownership.

## Final Target Structure

## 1. `app/` becomes state and action logic only

### `src/app/session/`

Owns pure application state and reducer logic.

Files:

- `mod.rs`
- `state.rs`
- `actions.rs`
- `effects.rs`
- `reducer.rs`
- `selectors.rs`
- `transcript.rs`
- `approvals.rs`
- `ask_user.rs`
- `models.rs`

Responsibilities:

- session/domain state only
- transcript entries and pending reply state
- session history and token estimate bookkeeping
- model selection state
- approval request data as plain structs
- ask-user request data as plain structs
- command semantics
- effect emission from actions
- pure derived view decisions

Must not depend on:

- `ratatui`
- `crossterm`
- `ratatui_textarea`
- `rig`

### `src/app/ui/`

Owns TUI-only ephemeral/editor/view state.

Files:

- `mod.rs`
- `state.rs`
- `composer.rs`
- `history.rs`
- `approvals.rs`
- `ask_user.rs`
- `picker.rs`

Responsibilities:

- `TextArea` ownership
- cursor and wrapping state
- viewport state
- render caches
- history selection anchors/focus
- shell approval editor state
- ask-user detail editor state
- picker/editor-only selection state

Must not own:

- workflow transitions
- transcript truth
- planning lifecycle
- model/business decisions

### `src/app/mod.rs`

Thin integration façade only.

Exports:

- `AppShell`
- `AppAction`
- `AppEffect`
- `SessionState`
- `UiState`

`AppShell` contains:

- `session: SessionState`
- `ui: UiState`

No business logic should accumulate here.

## 2. `runtime/` owns orchestration

Create `src/runtime/` and move all runtime control flow there.

Files:

- `mod.rs`
- `bootstrap.rs`
- `tui.rs`
- `headless.rs`
- `effect_executor.rs`
- `reply_driver.rs`
- `clipboard.rs`
- `command_history.rs`

Responsibilities:

- bootstrapping app/services from config
- TUI event loop
- headless prompt execution loop
- effect execution
- active reply task lifecycle
- cancel/resume behavior
- deferred failure handling
- terminal clipboard integration
- command history persistence integration

Dependency rule:

- `runtime/*` may depend on `app`, `llm`, `features`, `tools`, `stats`, `config`
- nothing outside runtime may depend on `runtime/*` except crate entrypoints

## 3. `llm/` becomes provider integration only

Replace `src/llm.rs` with `src/llm/`.

Files:

- `mod.rs`
- `service.rs`
- `types.rs`
- `agent_builder.rs`
- `streaming.rs`
- `resume.rs`
- `compaction.rs`
- `safety.rs`
- `hooks/mod.rs`
- `hooks/write_approval.rs`
- `hooks/shell_approval.rs`
- `hooks/ask_user.rs`
- `hooks/capture.rs`

Responsibilities:

- provider client creation
- prompt execution/streaming
- step-boundary/resume handling
- history compaction
- shell-risk classification
- approval/ask-user streaming hooks
- event emission to runtime

Must not depend on:

- `ui/*`
- renderer code
- TUI widgets
- planning UI/view logic

Allowed dependencies:

- `config`
- `agent`
- `completion_request`
- `stats`
- `tools::catalog`
- shared domain enums for approvals/access mode

## 4. `features/planning/` becomes a vertical feature slice

Create `src/features/planning/`.

Files:

- `mod.rs`
- `protocol.rs`
- `state.rs`
- `transitions.rs`
- `executor.rs`
- `view.rs`

Responsibilities:

- planning wrapper tags
- planning prompts
- planning-agent sanitization/defaulting
- planning job derivation
- planning session state machine
- plan review state
- planner fanout and synthesis execution
- planning-specific view model generation

### Planning state ownership

Planning becomes one owned feature state under session state.

Target states:

- `Idle`
- `Drafting`
- `Conversation`
- `RunningFanout`
- `Finalizing`
- `Review`

`Review` is part of planning state, not a separate app-global pending mode.

### Planning transitions must be explicit

Create named transitions such as:

- `enter_draft`
- `cancel_draft`
- `start_conversation`
- `accept_brief_and_start_fanout`
- `start_finalization`
- `show_review`
- `accept_review_for_implementation`
- `request_review_changes`
- `clear_planning`

No planning stage mutation may happen outside planning transition helpers.

### Planning executor boundary

`features/planning/executor.rs` owns:

- spawning planner batches
- collecting planner results
- synthesizing final plan
- mapping executor results back into app/runtime events

After cutover, no planner fanout logic remains in crate root or general runtime helpers.

## 5. `tools/` gets a single catalog boundary

Keep concrete tools where they are, but add an explicit catalog.

Files:

- `src/tools/catalog.rs`
- existing tool implementations remain

`tools/catalog.rs` owns:

- enabled-tool selection by context
- mutation classification
- shell-tool classification
- user-visible tool metadata needed by approvals and resume matching

`llm` must consume tool metadata through the catalog only.

Concrete tool modules must not leak policy/classification logic across the codebase.

## 6. `ui/` becomes rendering only

Keep `src/ui/`, but reduce it to rendering and view helpers.

Files may remain similar, but with these rules:

- renderer consumes session selectors and planning/view models
- renderer may update UI caches/state only
- renderer may not mutate domain/session state
- renderer may not perform workflow transitions
- input mapping may inspect UI state, but must emit `AppAction` only

## Dependency Rules

These are mandatory and should be enforced by code review during the refactor.

### Allowed dependency direction

- `main.rs` -> `lib` public entrypoints
- `lib.rs` -> `runtime/*`
- `runtime/*` -> `app/*`, `llm/*`, `features/*`, `tools/*`, `config`, `stats`
- `ui/*` -> `app/session/selectors`, `app/ui/state`, `features/planning/view`
- `llm/*` -> `tools/catalog`, shared domain enums/types, `config`, `stats`
- `features/planning/*` -> `app/session` pure types and shared infra
- `tools/*` -> shared config/policy/domain types only

### Forbidden dependencies

- `app/session/*` -> any TUI crate
- `app/session/*` -> `llm/*`
- `app/session/*` -> `runtime/*`
- `features/planning/state.rs` -> TUI or runtime modules
- `ui/*` -> `llm/*`
- `llm/*` -> `ui/*`
- `tools/*` -> `features/planning/*`
- `lib.rs` -> detailed planning executor logic
- crate root -> direct orchestration logic after cutover

## Concrete Refactor Decisions

## 1. Session/UI split details

### Move into `SessionState`

- transcript entries
- transcript revision/versioning
- session history
- estimated history tokens
- pending reply metadata and accumulated output
- mode and approval mode
- selected model/safety/planning-agent configuration
- stats snapshot
- pending write approval data as plain values
- pending shell approval data as plain values
- pending ask-user data as plain values
- planning feature state
- command recall data if it is logical history, not editor widget state

### Move into `UiState`

- composer `TextArea`
- composer wrap width
- composer visual column
- composer layout cache
- history viewport/render snapshot
- history selection UI state
- history render cache
- shell approval edit mode and text editors
- ask-user detail edit mode and text editors
- picker-local UI selection state

### Required behavior change in implementation style

Current pending ask-user and approval structs mix domain truth with editor widgets. That must stop.

New pattern:

- session stores the request and selected answer IDs
- UI stores active editor text and edit mode
- reducer resolves selected IDs and plain strings
- renderer reads both and builds the panel

## 2. Reducer and effect model

Replace current “state object with lots of methods” style with:

- `AppAction`
- `AppEffect`
- `reduce(session, action) -> Option<AppEffect>`
- side-effect-free selectors for derived labels and visibility

Permitted exceptions:

- `UiState` helper methods for editor/cursor mechanics
- small façade methods on `AppShell` for ergonomics

Not permitted:

- business transitions hidden inside renderer
- business transitions hidden in runtime
- cross-feature mutation through ad hoc helper methods scattered on `App`

## 3. Runtime cutover shape

### `runtime/bootstrap.rs`

Builds:

- config
- session state
- UI state for TUI mode
- stats store
- subagent manager
- command history store
- `LlmService`

### `runtime/tui.rs`

Owns:

- poll loop
- draw timing
- stream/subagent event intake
- action dispatch
- effect execution calls

### `runtime/headless.rs`

Owns:

- one-shot headless prompt flow
- text output assembly
- event filtering for headless mode

### `runtime/reply_driver.rs`

Owns:

- active reply task handle
- task abort/cancel
- resume after approvals/ask-user
- defer-failed-stream-event logic

### `runtime/effect_executor.rs`

Owns:

- handling of all `AppEffect`s
- config writes
- llm rebuilds
- stats rotation
- clipboard writes
- planning executor invocation

## 4. LLM cutover shape

### `LlmService` remains the façade

Keep one façade type so callers do not need to know module internals.

`LlmService` public responsibilities:

- `from_config`
- prompt streaming entrypoints
- compaction entrypoint
- approval resolution entrypoints
- ask-user resolution entrypoints
- controller accessors

Everything else moves behind internal modules.

### Specific decomposition

- move all hook implementations into `llm/hooks/*`
- move shell-risk prompt and parsing into `llm/safety.rs`
- move replay probe and override matching into `llm/resume.rs`
- move compaction budget/rebuild helpers into `llm/compaction.rs`
- move `build_agent` and preamble assembly into `llm/agent_builder.rs`

After cutover, `service.rs` should be orchestration over smaller modules, not another monolith.

## 5. Planning cutover shape

### `protocol.rs`

Owns:

- constants for planning tags
- prompt builders
- planning job derivation
- sanitization/defaulting of planning agents

### `state.rs`

Owns:

- planning state enum
- review mode state
- planning data structures stored in session state

### `transitions.rs`

Owns:

- all legal planning state changes
- guard logic around transitions
- helper functions used by reducer

### `executor.rs`

Owns:

- planner subagent batch spawn
- result collection
- synthesis prompt execution

### `view.rs`

Owns:

- view model for footer/status/prompt review panels
- strings/labels derived from planning state

## 6. Crate-root cleanup

Final `src/lib.rs` should contain only:

- module declarations
- public entrypoints
- terminal setup/restore helpers
- minimal wiring to runtime modules

It must not contain:

- event loop logic
- planning executor logic
- effect executor logic
- reply-task orchestration
- workflow-specific helpers beyond tiny forwarding functions

## Implementation Sequence With Hard Checkpoints

## Phase 1. Create structure and move code mechanically

Actions:

- create `runtime/`, `llm/`, `features/planning/`, `app/session/`, `app/ui/`
- move code without changing behavior
- keep temporary re-exports only where needed for compile progress

Checkpoint:

- project compiles
- behavior unchanged
- `src/llm.rs` and `src/lib.rs` may still be transitional, but new directories exist

## Phase 2. Split domain state from UI state

Actions:

- introduce `SessionState`, `UiState`, `AppShell`
- move TUI widget state out of domain structs
- convert pending approval and ask-user data to pure structs
- convert renderer to read session + UI separately

Checkpoint:

- domain/session modules compile without TUI imports
- headless path no longer constructs UI widget state
- renderer no longer mutates session/domain state

## Phase 3. Purify reducer ownership

Actions:

- replace scattered `App` business methods with reducer + selectors
- keep UI helper methods limited to editor mechanics
- move derived labels/percentages/visibility checks into selectors

Checkpoint:

- workflow transitions are driven by reducer/effects
- no domain transition lives in renderer
- session behavior can be tested without TUI state

## Phase 4. Extract runtime orchestration

Actions:

- move TUI loop to `runtime/tui.rs`
- move headless flow to `runtime/headless.rs`
- move effect handling to `runtime/effect_executor.rs`
- move reply-task lifecycle to `runtime/reply_driver.rs`

Checkpoint:

- `src/lib.rs` is façade-sized
- TUI and headless boot through runtime modules
- no planning or reply-task orchestration remains in crate root

## Phase 5. Decompose LLM internals

Actions:

- split `llm` into service/builder/hooks/safety/resume/compaction/streaming
- add `tools/catalog.rs`
- redirect `LlmService` to consume tool metadata only through catalog

Checkpoint:

- `src/llm.rs` is deleted
- no LLM file remains monolithic
- LLM has no TUI dependencies

## Phase 6. Consolidate planning

Actions:

- move planning protocol into `features/planning/protocol.rs`
- move planning state and review state into planning feature
- move fanout/synthesis executor into planning executor
- move planning-specific render data into planning view model

Checkpoint:

- no planning executor logic exists outside `features/planning/*`
- no planning stage mutation exists outside planning transitions/reducer integration
- review prompt state is part of planning state, not app-global stray state

## Phase 7. Tighten final public surface and remove shims

Actions:

- remove temporary re-exports
- make internal modules crate-private where possible
- retain only necessary public entrypoints

Checkpoint:

- final module graph matches target architecture
- no compatibility shims remain unless they are intentional

## Test Plan

## Unit tests to add or migrate

### Session state

- submit message with normal text
- ignore empty submit
- cancel pending reply
- update session history on completion
- failed stream clears pending reply state correctly
- model selection updates session state correctly
- approval queue enqueue/dequeue behavior
- ask-user selection and completion behavior

### Planning

- planning agent sanitization
- planning job derivation includes main model first
- planning tag extraction/stripping
- legal stage transitions
- illegal transition guard behavior
- review acceptance and review-feedback transitions
- fanout collection success/failure aggregation

### Tools catalog

- enabled tool set by access mode and role
- mutation classification
- shell tool classification
- main-vs-subagent tool visibility

### Runtime

- headless runtime assembles only text output
- deferred failed stream events work correctly
- reply resume after write approval
- reply resume after shell approval
- reply resume after ask-user response

### Renderer/UI

- plan review rendering from planning view model
- approval panels render from pure request data + UI editor state
- history/composer rendering still behaves correctly
- renderer does not require mutable session/domain state

## Boundary-oriented proof

The refactor is only complete if these are true:

- pure session/planning tests do not import any TUI crates
- headless runtime tests do not instantiate `TextArea`
- planning executor tests do not require full TUI runtime
- tool catalog tests do not depend on `LlmService`

## Final verification

Run:

```bash
cargo fmt --check
cargo test
```

## Review Standard Before Calling It Done

Before merging, perform a critical pass against these questions:

- Did we actually remove monolithic ownership, or only rename it?
- Is any file still acting as a hidden god object?
- Are planning changes localized now?
- Can headless logic run without carrying TUI concerns?
- Does any module depend “upward” against the declared dependency direction?
- Did we introduce generic abstractions that do not pay for themselves?
- Is `lib.rs` truly just a façade?
- Is `LlmService` now a provider façade rather than a catch-all runtime?

If any answer is “not clearly,” the refactor is not complete.

## Assumptions

- CLI compatibility is the only required external compatibility target.
- A single reshape is preferred over a long migration of compatibility layers.
- Internal module names may vary slightly, but ownership and dependency rules above are mandatory.
- The best outcome is a strict architectural cutover, not a half-step incremental cleanup.
