# APP-6429 — Local prompt steering technical design

See [PRODUCT.md](PRODUCT.md) for numbered behavior. Research is pinned to Warp commit
`96d816ad2b3881418b56d946e70bfb004e89bd03`. This PR contains specs only; implementation waits for
Harry's approval.

## Context

- [`settings/ai.rs (908-1050)`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/settings/ai.rs#L908-L1050)
  defines the synced GUI `PromptSubmissionMode` (Interrupt/Queue) and the independent LRC policy.
  Neither is an Interrupt/Steer preference.
- [`queued_query.rs (230-295)`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/ai/blocklist/queued_query.rs#L230-L295)
  stores delivery mode on the conversation, not the row. Native/Oz binding selects Steering in
  [`controller/startup_queue.rs`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/ai/blocklist/controller/startup_queue.rs).
  Exposing that enum directly would also steer explicitly response-queued rows.
- [`controller.rs (1851-1958)`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/ai/blocklist/controller.rs#L1851-L1958)
  builds one ready prompt as `AIAgentInput`; tool-result and orchestration follow-ups call it.
  [`can_dispatch_queued_warp_agent_prompt`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/ai/blocklist/controller.rs#L3269-L3288)
  guards fresh dispatch against active streams, setup, subagents, and exiting conversations.
- [`input.rs (14102-14150)`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/terminal/input.rs#L14102-L14150)
  handles empty-input send-now before queueing, setup, slash commands, and regular submission.
  [`maybe_queue_input_for_in_progress_conversation`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/terminal/input.rs#L14763-L14918)
  already captures row attachments and enforces queue, compaction, and LRC precedence.
- [`input.rs (4468-4605)`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/terminal/input.rs#L4468-L4605)
  resolves local versus remote submission. Native driver ownership is defined in
  [`conversation_driver.rs`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/ai/agent/conversation_driver.rs).
- [`terminal/view.rs (5614-5791)`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/terminal/view.rs#L5614-L5791)
  handles idle fallback and clean/error/cancellation draining. LRC draining follows separately.
- [`warp_agent_page.rs (3871-3942)`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/settings_view/warp_agent_page.rs#L3871-L3942)
  supplies the dropdown pattern; its palette bindings are at lines 365-424.
  [`queued_prompts_panel.rs (1200-1269)`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/terminal/view/queued_prompts_panel.rs#L1200-L1269)
  supplies the command prefix and italic timing-suffix pattern.

## Proposed changes

### Setting and rollout

- Add `UnqueuedPromptBehavior::{Interrupt, Steer}` beside `PromptSubmissionMode`, with Interrupt as
  default, and an `AISettings::unqueued_prompt_behavior` field. Follow the existing enum setting
  macro: GUI surface, all desktop platforms, global cloud sync respecting the sync preference.
  Use TOML path `agents.warp_agent.other.unqueued_prompt_behavior`.
- Add `FeatureFlag::AgentPromptSteering` in `crates/warp_features/src/lib.rs` and `DOGFOOD_FLAGS`
  only. Do not add it to default Cargo features, `DEBUG_FLAGS`, `LOCAL_FLAGS`, `PREVIEW_FLAGS`, or
  `RELEASE_FLAGS`. Local/dev binaries already include dogfood flags; staging-connected internal
  builds use this same channel mechanism. There is no distinct staging flag list.
- Gate the setting registration, Settings widget, palette actions, and new capture path with the
  same flag. The existing queue-panel capability (`QueueSlashCommand`) is also required; do not
  capture rows when their panel is disabled. A synced value on a public channel must not activate
  capture.
- Use the existing dropdown lifecycle, change-event subscription, and `set_value` action in
  `settings_view/warp_agent_page.rs`. Keep the new dropdown visible independently of the existing
  Interrupt/Queue selection; its description explains queue precedence.
- Add per-value context flags in `settings_view/mod.rs`, compute them in `workspace/view.rs`, and
  register the two `FixedBinding`s with the existing AI-enabled predicate.

### Per-row timing without changing remote delivery

- Add a timing field to `QueuedQuery`, separate from origin and prompt/command kind:
  `InheritConversation`, `AfterResponse`, or `NextTurn`. Keep existing constructors defaulting to
  `InheritConversation` so native startup, Oz, and shared-session injection retain their behavior.
- Only the new flagged local policy assigns explicit timing. Mark ordinary local steering rows
  `NextTurn`; mark local explicit `/queue`, auto-queue, LRC, and compaction-follow-up rows
  `AfterResponse`. LRC origin and snapshot-lock checks still govern their earlier command-finish
  path; `AfterResponse` must not suppress that path.
- Add a local steering origin for existing `QueuedPrompt*` telemetry and exhaustive origin matches.
  Do not overload origin as the timing selector. Do not add a server schema or persist queue rows.
- Keep one row vector and stable row IDs. Editing, reordering, failed-send restoration, and
  attachment access retain timing on the row. Preserve inherited rows and all locked-row rules.
- Add a non-mutating natural-boundary selector. For ordinary local conversations it finds the
  first `NextTurn` row in visible order. Select the first such row before checking its readiness
  and edit state: an unready earlier steering row blocks later steering rows. `AfterResponse`
  rows do not block that lane.
- Native setup and running queued-command barriers block dispatch globally. Origin-specific
  protections still apply. Do not scan past a locked safety barrier to obtain a steering row.
- Preserve the old FIFO-head selector for inherited conversation-level steering and primary
  shared-session injections. Do not interpret a local preference as permission to alter those
  paths. Full-response completion and manual empty-input Enter still use visible queue order.

### Submission and native request integration

- Add the steering intercept after the existing queue/setup and slash-command handling, before
  ordinary AI submission. Share the current capture operations for submitted-input telemetry,
  editor clearing, and attachment transfer instead of introducing another attachment store.
- Eligibility requires GUI local routing, no ambient/Oz task context, native driver ownership, a
  nonempty existing conversation in progress, AI rather than shell input, no detected slash/skill
  command, the feature flag, and Steer. Resolve routing with `resolve_ai_query_routing`; driver
  ownership alone does not establish local scope. Preserve all earlier safety checks.
- Do not set the conversation's delivery mode when the setting changes. Extend
  `steer_head_prompt_for_request` to accept a local next-turn candidate as well as its legacy
  inherited candidate. Leave primary-injection command-interruption behavior confined to its
  existing shared-session path.
- Reuse the tool-result and orchestration request hooks. Include one row with the root task's
  user query without cancelling ongoing work or resetting the live input context. Do not inject
  into an active subagent or a request that starts a server subagent; defer to a safe main-agent
  boundary after that subagent settles.
- Extend idle dispatch to recognize explicit next-turn rows without setting the whole queue to
  Steering. Consult `can_dispatch_queued_warp_agent_prompt` rather than treating “no stream” as
  “idle”; pending actions and subagents are not clean completion.
- Keep the completion, LRC, edit-restoration, and error/cancellation drain paths separate. Reuse
  `remove_fired_row` by ID after dispatch acceptance. If a synchronous request rejection occurs
  after extraction, restore the same row at its previous position with its ID, timing, and
  attachments. Never create a second row for the same pending dispatch.

### Shared panel

- Pass timing into `RenderRowProps`. Use the existing theme-derived italic suffix element for
  Next turn / After response; LRC suffix and command-prefix rules take precedence.
- Keep the suffix outside the preview-versus-editor branch so it is visible during editing.
  Shrink or clip the preview/editor before hiding timing. Keep row action widths stable.
- Do not group or sort rows by kind. Existing drag changes visible order; automatic selection
  respects that order within a lane. Send-now and empty-input Enter retain their current
  explicit-override handlers.
- Refresh on setting changes and relevant queue mutations. Capture, drain, and rendering must
  not acquire a second terminal-model lock through a caller that already holds it.

## Decisions and trade-offs

- **Separate preference, not a third value in Interrupt/Queue.** The existing setting and chip
  choose full-response queueing. Replacing either would remove an independent user control.
- **Per-row timing, not a conversation-mode toggle.** Slightly more state prevents explicit
  queue rows from being consumed at the next request. It also preserves timing after setting
  changes and permits mixed rows.
- **FIFO per kind, not global FIFO.** Harry chose this so steering corrections are not held
  behind response work. The visible queue remains familiar; suffixes explain the exception.
- **One shared panel, not a steering buffer.** Harry requested the existing controls. A second
  buffer would duplicate edit, attachment, and recovery behavior.
- **Local ordinary prompts first.** Preserve existing command/skill expansion and remote delivery
  rather than emulate steering by cancelling and re-prompting another transport.

## Testing and validation

The following names describe required new test cases, not results already obtained.

- `queued_query_tests.rs`: `next_turn_skips_after_response_but_preserves_lane_order`,
  `editing_next_turn_blocks_only_its_lane`, `next_turn_respects_safety_barriers`,
  `row_timing_survives_edit_reorder_and_restore`, and
  `inherited_native_delivery_is_unchanged` cover PRODUCT 10, 16-18, 22, 25, and 28-29.
- `terminal/input_tests.rs`: parameterized `unqueued_prompt_behavior_routing` covers flag on/off,
  Interrupt/Steer, idle/busy, queue chip/default overrides, command, slash/skill, local/remote,
  native/external driver, and LRC/summarize cases (PRODUCT 1, 3, 5-12, 15). Assert no cancellation
  or immediate request on capture and assert attachment ownership.
- Add controller tests beside existing `controller/startup_queue_tests.rs` for
  `local_steering_piggybacks_once`, `response_queue_never_piggybacks`,
  `local_steering_waits_for_main_agent_boundary`, `steering_falls_back_on_clean_idle`, and
  `rejected_steering_send_restores_row`. Exercise both tool-result and orchestration hooks.
  Assert exact request user-query order, preserved tool results, and no duplicate row delivery
  (PRODUCT 13-18, 24, 28).
- `terminal/view/queued_prompts_tests.rs` and input host tests: exercise mixed-kind completion,
  send-now, empty-input Enter, LRC snapshot unlock/command finish, inline edit, error/cancel,
  draft/attachment preservation, pane switches, and conversation deletion (PRODUCT 10, 17, 23-27).
- Settings and panel tests cover enum default/serialization, setting-change re-sync, palette
  contexts, disabled-flag visibility, and layout with mixed rows, a long preview, and an editor
  (PRODUCT 1-5, 19-22, 29). Follow existing separate `*_tests.rs` conventions.
- `crates/warp_features/src/features_tests.rs`:
  `agent_prompt_steering_is_dogfood_only_by_default` asserts membership in `DOGFOOD_FLAGS` and
  absence from every other list. Verify dev/local include dogfood and Preview/Stable/Oss do not.
- Run affected `cargo nextest run -p warp` filters for queue, new input/controller, Settings, and
  panel tests; run `cargo nextest run -p warp_features`. Run targeted
  `cargo clippy -p warp -p warp_features --all-targets --tests -- -D warnings` and build the GUI
  candidate with `cargo build -p warp --bin warp`. Finish code changes with `./script/format`
  once. Do not run full presubmit unless explicitly requested.
- After a successful build, use the opted-in computer-use verification on the running app.
  Record changing the setting, keeping the queue chip unchanged, and submitting a mixed queue
  during multi-tool work. Show the Next turn rows dispatching at natural boundaries while an
  After response row remains, then show that row dispatching after clean completion. Also
  capture the static Settings and mixed-panel states. Verify long text, inline editing, and
  theme readability. Report the observed OS and attach video/screenshots to the implementation
  PR. Do not claim platform coverage that was not observed.

## Implementation boundary

Keep this work on `factory/app-6429-interrupt-steer` and the same draft PR. The setting, row model,
and controller are tightly coupled; one worker should implement them. No server or protocol PR
is needed. Spec approval is the next human gate.
