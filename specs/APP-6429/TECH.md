# APP-6429 — Local prompt steering technical design

See [PRODUCT.md](PRODUCT.md) for behavior and feedback points. This is an implementation outline,
not approval to implement. Source references use commit `96d816ad2b3881418b56d946e70bfb004e89bd03`.

## Context
Native steering already exists, but
[`QueuedPromptDeliveryMode`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/ai/blocklist/queued_query.rs#L230-L295)
applies to a whole conversation. Simply exposing it would also steer `/queue` and auto-queued rows.
Reuse the existing
[`request-boundary hook`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/ai/blocklist/controller.rs#L1851-L1958),
[`input capture`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/terminal/input.rs#L14763-L14918),
[`Settings dropdown`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/settings_view/warp_agent_page.rs#L3871-L3942),
and [`row suffix`](https://github.com/warpdotdev/warp/blob/96d816ad2b3881418b56d946e70bfb004e89bd03/app/src/terminal/view/queued_prompts_panel.rs#L1200-L1269).

## Proposed changes
- **Setting and flag:** add `UnqueuedPromptBehavior::{Interrupt, Steer}` to `settings/ai.rs`, using
  the existing synced GUI-setting pattern and TOML key
  `agents.warp_agent.other.unqueued_prompt_behavior`. Add `AgentPromptSteering` to
  `crates/warp_features/src/lib.rs` and `DOGFOOD_FLAGS` only; internal staging builds use that list.
  Gate Settings, palette actions, and capture together; require the queue-panel flag too.
- **Row timing:** add `InheritConversation / AfterResponse / NextTurn` to `QueuedQuery`, separate
  from origin and prompt/command kind. Existing remote/native-startup constructors inherit today's
  behavior. The new local policy assigns explicit timing and a steering telemetry origin.
  Keep one vector, stable IDs, attachments, and timing through edits/reorders/restoration.
- **Capture:** after existing queue/setup/slash handling, capture an ordinary busy local prompt
  when Steer is selected. Require GUI local routing, native driver ownership, and no ambient/Oz
  task context; ownership alone does not prove locality. Reuse input clearing and attachment capture.
- **Dispatch:** the native tool-result/orchestration hooks select the first NextTurn row, not the
  first ready row of any kind. An edited/unready steering head blocks later steering rows.
  Preserve setup, queued-command, LRC snapshot, and main-agent/subagent safety barriers. Keep
  LRC command-finish delivery and inherited remote injection paths unchanged.
- **Completion and recovery:** use `can_dispatch_queued_warp_agent_prompt` for idle fallback;
  “no stream” alone is insufficient. Completion and empty-input Enter retain visible-order
  selection; row Send now targets the selected ID. Remove a row only after dispatch acceptance
  or restore it on rejection. Preserve current error/edit recovery and live drafts; do not
  introduce nested terminal-model locks.
- **UI:** wire the dropdown, change subscription, palette actions/context flags, and search terms
  in `warp_agent_page.rs`, `settings_view/mod.rs`, and `workspace/view.rs`. Pass timing to
  `queued_prompts_panel.rs`; keep suffixes visible beside both clipped previews and inline editors.

## Testing and validation
- **Routing/settings (PRODUCT 1-3, 7):** input/Settings tests cover flag off/on, default/synced
  selection, queue/LRC/compaction precedence, idle/busy state, and excluded transports/commands.
  `agent_prompt_steering_is_dogfood_only_by_default` checks channel-list membership.
- **Ordering/recovery (PRODUCT 3-6):** queue/controller/terminal host tests cover mixed FIFO, both
  natural-request hooks, clean completion, edits/readiness, send-now, error/cancel, attachments,
  draft/conversation isolation, and failed or racing dispatch without loss/duplication. Add a panel
  layout test for mixed rows, long text, and inline editing.
- **Commands:** run affected `cargo nextest run -p warp` filters and
  `cargo nextest run -p warp_features`, then
  `cargo clippy -p warp -p warp_features --all-targets --tests -- -D warnings` and
  `cargo build -p warp --bin warp`. Finish with `./script/format` once; no full presubmit.
- **Opted-in visual proof:** after the GUI build succeeds, record changing the setting and a
  mixed queue where steering dispatches at a natural boundary while response work waits.
  Include Settings/panel screenshots, check long text and editing, and report the observed OS.
  Attach proof to this PR; these are required checks, not results already obtained.
