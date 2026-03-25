# Refactor Plan: App State, Reducer Boundaries, and Planning Isolation

## Summary
Refactor the app around three explicit boundaries:

- A typed app-state boundary that separates domain state from UI/editor state and routes input through a single authoritative interaction target.
- A thin `App` shell with feature-local reducers and query helpers, replacing the current `App`/`ReducerContext` façade sprawl.
- A typed planning subsystem that owns all planning protocol parsing, stores typed planning artifacts, and uses event-driven subagent waiting instead of polling.

End state requirements for this plan:
- `src/app/session/reducer.rs` is a small dispatcher, not the place where behavior lives.
- `ReducerContext` is removed or reduced to a tiny state wrapper with no feature logic.
- `App` keeps constructors and `apply`, but no longer exposes dozens of wrapper getters/setters.
- No code outside `src/features/planning` parses or strips planning tags.
- Planning fanout no longer uses `sleep` + `inspect` polling.

## Implementation Changes

### 1. Introduce explicit app-state and interaction routing
- Add a crate-level `AppState` bundle under `src/app` that owns `session: SessionState` and `ui: UiState`.
- Slim `SessionState` to domain/session concerns only:
  - access mode, approval mode, transcript/history, pending reply, model/safety selection, planning domain state, approvals queues, stats, workspace root
  - remove editor-only concerns such as command recall from `SessionState`
- Move editor/input-only concerns into `UiState`:
  - composer buffer/layout state
  - command recall state
  - picker selection
  - plan review selection
  - ask-user tab/editor state
  - shell-approval editor state
  - history pin/render cache
- Add a single authoritative input-routing enum, for example `InputTarget`, with exactly these variants:
  - `Composer`
  - `CommandPalette`
  - `Picker`
  - `ShellApprovalSelection`
  - `ShellApprovalEditor`
  - `AskUserSelection`
  - `AskUserEditor`
  - `PlanReviewSelection`
- Add one query function `active_input_target(&AppState) -> InputTarget` and require all input actions to route through one `match` on that enum.
- Replace the repeated priority chains in `SelectPreviousCommand`, `SelectNextCommand`, `Editor`, `Paste`, and `SubmitMessage` with target-specific handlers keyed off `InputTarget`.
- Keep current keyboard behavior unchanged unless the current behavior is an artifact of the old boolean chain. The intended preserved behavior is:
  - approval/review overlays preempt composer input
  - picker preempts command palette
  - command palette only applies when composer content starts with `/`
  - composer recall only applies when no overlay target is active

### 2. Split reducer behavior by feature and remove the giant context object
- Replace the current monolithic reducer layout with feature reducers under `src/app/reducer/`:
  - `system.rs` for quit, mode toggle, tick, session reset
  - `input.rs` for editor, paste, navigation, submit routing
  - `history.rs` for scroll and selection
  - `approvals.rs` for write/shell approval actions
  - `ask_user.rs` for ask-user actions
  - `planning.rs` for plan review and planning-mode actions
  - `events.rs` for stream and subagent events
- Keep `apply(state, action) -> Option<Effect>` as the single entrypoint, but make it a thin dispatcher into these modules.
- Replace `ReducerContext` with a small mutable state handle, e.g. `AppStateMut`, containing only shared low-level helpers that genuinely need both session and UI access:
  - append transcript entries
  - replace session history
  - clear or seed composer
  - resume/pin history
  - open/close overlays
- Do not allow feature logic to accumulate in this shared helper. Feature rules must live in the feature reducers/modules named above.
- Group actions by concern so the main reducer no longer carries a giant flat behavioral match. Either:
  - introduce nested action enums (`InputAction`, `HistoryAction`, `PlanningAction`, `EventAction`, `SystemAction`), or
  - keep the current flat enum but route each group immediately into its feature module.
- Make feature modules own their invariants. Examples:
  - approvals module owns approval queue advancement and overlay open/close behavior
  - planning module owns planning-stage transitions and review-state transitions
  - input module owns routing from `InputTarget` to editor/navigation behavior

### 3. Shrink `App` into a composition root and move reads into query helpers
- Keep `App::new`, `App::with_startup`, and `App::apply`.
- Replace the current `App` wrapper API with:
  - `app.state()` / `app.state_mut()` accessors for internal crate use, or crate-private field access through `AppState`
  - query helpers under `src/app/query.rs` or feature-local query modules
- Move read-only derived logic out of `App` methods into query helpers, including:
  - pending-approval presence and request-id lookups
  - plan review active/feedback active checks
  - command palette visibility and selected command
  - current model info / supported reasoning levels
  - history busy indicator and status label
  - overlay height / command palette height / selected plan review index
- Update runtime and render code to use `AppState` + query helpers instead of calling dozens of `App` methods.
- Remove the current `app/shell/*` façade modules once their logic has moved into queries or reducers. `App` should become a small composition type, not a parallel API surface over session and UI.
- Acceptance criterion for this section:
  - `src/app/shell/mod.rs` is reduced to app construction plus a very small public surface
  - there is no second large “logic object” after `ReducerContext` is removed

### 4. Replace stringly planning flow with typed planning artifacts
- Extend `PlanningFeatureState` so it stores typed planning artifacts instead of only stage/review:
  - `normalized_brief: Option<PlanningBrief>`
  - `proposed_plan: Option<ProposedPlan>`
- Add typed planning result types under `src/features/planning`, for example:
  - `PlanningBrief { markdown: String }`
  - `ProposedPlan { markdown: String, raw_block: String }`
  - `PlanningReply { ConversationText(String), ReadyBrief(PlanningBrief), ProposedPlan(ProposedPlan) }`
- Keep tag-based prompting if needed for model reliability, but make all parsing private to `src/features/planning/protocol.rs`.
- Replace exports such as `contains_proposed_plan`, `extract_planning_ready_brief`, `strip_planning_ready_tags`, and `strip_proposed_plan_tags` with one planning-owned parser API that returns `PlanningReply`.
- Update stream-finish handling so it does not inspect raw message text directly:
  - planning conversation completion parses into `ReadyBrief` and emits `Effect::RunPlanningWorkflow { brief }`
  - planning finalization parses into `ProposedPlan`, stores it in planning state, inserts a typed transcript entry, and enters review mode
  - non-planning replies remain normal transcript messages
- Replace transcript-level plan parsing with typed entries:
  - add `TranscriptEntry::ProposedPlan(ProposedPlanEntry)` or equivalent typed message kind
  - remove transcript utilities that scan message text for `<proposed_plan>`
- Update plan acceptance/revision flow to read `planning.proposed_plan` from state, not `latest_proposed_plan_message()` scanning transcript text.
- Keep the visible user-facing plan text unchanged: the review UI still shows “A proposed plan is ready”, and accepted-plan prompts still pass the plan content back to the model.

### 5. Move planning fanout to event-driven waiting
- Remove `collect_planning_batch_results()` polling via `inspect()` + `sleep(Duration::from_millis(100))`.
- Extend `SubagentManager` with a batch-completion API built on the existing watch-based waiting path, for example `wait_all(ids, timeout)`.
- `wait_all` must:
  - block until every requested subagent has left `Running`, or an inactivity timeout occurs
  - return final snapshots for all requested ids
  - preserve current inactivity-timeout semantics
- Update planning fanout to:
  - spawn one batch
  - wait for the whole batch with `wait_all`
  - separate successful outputs from failed/cancelled/inactive snapshots
  - proceed to finalization only after the full batch resolves
- Keep concurrency control unchanged: batching still uses `config.subagents.max_concurrent.max(1)`.

## Test Plan
- Reducer/input tests:
  - `active_input_target()` returns the correct target for each overlay/editor/review state
  - navigation, editor input, paste, and submit each dispatch through the correct target with no duplicated precedence logic
  - command recall only works in composer mode
- App/query tests:
  - query helpers reproduce current UI/runtime behavior for command palette visibility, busy indicators, overlay heights, and selection state
  - `App` no longer needs wrapper coverage beyond construction and `apply`
- Planning protocol tests:
  - parser returns `ReadyBrief` for normalized brief replies
  - parser returns `ProposedPlan` for final plan replies
  - raw tags are never required outside `src/features/planning`
  - malformed planning output produces a controlled “continue conversation / failed parse” path rather than leaking tags into unrelated modules
- Planning reducer/event tests:
  - conversation finish with `ReadyBrief` starts fanout
  - finalization finish with `ProposedPlan` enters review and stores the typed plan artifact
  - plan acceptance and revision prompts read from stored `ProposedPlan`
- Subagent waiting tests:
  - `wait_all` returns once all ids are terminal
  - inactivity timeout is reported when one id stalls
  - planning fanout no longer depends on polling or sleeps
- Regression checks:
  - `cargo fmt --check`
  - `cargo test`
  - focused review of `runtime/tui.rs` and `ui/render/*` to confirm no user-visible interaction regressions

## Assumptions and Defaults
- Keep the project as a single crate; this plan does not introduce a multi-crate split.
- Preserve all current user-facing commands, keyboard shortcuts, and planning UX unless a behavior exists only because of the current reducer precedence bug.
- Keep the current model prompt contract for planning if it is still needed for model compliance, but confine that contract entirely to `src/features/planning`.
- Do not change config file shape or persisted command-history format unless a refactor makes it strictly necessary; if a shape change becomes necessary, add a backward-compatible loader instead of a breaking change.
- A point is considered “fully solved” only when the old cross-cutting helpers are removed, not merely deprecated.
