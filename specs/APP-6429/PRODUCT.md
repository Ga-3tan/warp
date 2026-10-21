# APP-6429 — Interrupt or steer local agent prompts

## Summary

Add a setting that controls whether a prompt interrupts active agent work or waits for the next
turn. Keep full-response queueing separate, including its existing setting and auto-queue chip.
Reuse the queued-prompts panel to show both kinds of waiting prompts.

Issue: [APP-6429](https://linear.app/warpdotdev/issue/APP-6429).

## Design context

Figma: none provided. Harry requested reuse of the current controls.

Harry chose Interrupt by default, dogfood/staging-only availability, FIFO within each kind, and
local-only scope. He opted in to computer-use verification. This spec requires approval before
implementation.

## Behavior

### Setting and scope

1. Add **Unqueued prompt behavior** to the Warp Agent settings beside the existing prompt-submission
   settings. Its choices are **Interrupt** and **Steer**. Interrupt is the default.
2. Explain the choices below the setting: “For local Warp Agent prompts when auto-queue is off.
   Interrupt sends immediately. Steer waits for the next turn. Queued prompts still wait until the
   full response finishes.”
3. Persist the selection across restarts. Sync it across devices when the user's settings-sync
   preference permits. Changing it affects future submissions, not prompts that are already waiting.
4. Expose matching Command Palette actions to select Interrupt or Steer. Hide the action for the
   currently selected value. Settings search must find the control for “interrupt”, “steer”, and
   “next turn”.
5. Put the setting and new submission behavior behind one feature flag. Enable it only for dogfood
   builds, including staging-connected internal builds. Do not enable it for Preview, Stable, or
   the open-source channel.
6. Apply the setting only to ordinary AI prompts in native, local desktop Warp Agent conversations.
   Cloud panes, shared-session viewers, remote child conversations, third-party agent harnesses,
   CLI rich input, the TUI, and shell-command submissions keep their current behavior.
7. Slash commands and skills keep their current handlers and timing. In particular, `/queue` remains
   an explicit full-response queue operation, and action commands do not become steering text.

### Submission and timing

8. The existing **Default prompt submission mode** setting and auto-queue chip retain their current
   values, appearance, shortcut, and per-conversation override behavior. Do not add an input chip
   for Interrupt versus Steer.
9. Full-response queueing takes precedence over the new setting. When it is enabled, an ordinary
   prompt waits until the complete response finishes, even when Steer is selected.
10. Existing long-running-command, pending-command-snapshot, compaction, and setup protections retain
    precedence. The new setting must not release a protected prompt early or interrupt a command
    merely to create a steering opportunity.
11. With those policies inactive, Interrupt submits immediately through the current interrupt path.
    It does not create a waiting row.
12. With those policies inactive and native local work in progress, Steer captures the prompt in the
    existing queue panel. It clears the submitted input and transfers that prompt's attachments to
    its row. It does not cancel the active stream, tool, or subagent.
13. A steering prompt waits until the agent can make its next natural request, such as continuing
    after tool results. It is not injected into text that is already streaming. “Next turn” does
    not mean “after the entire response”.
14. Deliver at most one waiting steering prompt per natural request. Each prompt remains a distinct
    user message; do not concatenate prompts.
15. If no further natural request occurs, a clean full-response completion can deliver the next
    waiting prompt. If the agent is already idle when the user submits, send normally without
    leaving a stranded steering row.
16. Keep FIFO within each kind unless the user explicitly reorders or sends a row now. At a natural
    request boundary, the oldest steering row may bypass older full-response rows. It must not
    bypass an earlier steering row that is being edited or is not ready.
17. On clean full-response completion, use the existing panel order to choose the next row because
    both timing kinds are then eligible. Preserve command-in-flight, editing, attachment-readiness,
    and setup protections. Start only one next prompt through this completion path.
18. Example: the panel contains `/queue write tests`, then steering prompts “do not modify X” and
    “use the existing helper”. The next two natural requests receive the steering prompts in that
    order. “write tests” does not run until the response has fully completed.

### Panel and user control

19. Show both kinds in the existing panel in submission order. Give steering rows an italic
    **(next turn)** suffix and full-response prompt rows an italic **(after response)** suffix.
    Use the current row-suffix styling and theme colors, not a new control.
20. Keep the existing **(queued until the command finishes)** treatment for command-dependent
    prompts. Preserve the blue `!` treatment for shell-command rows. Do not relabel remote or
    shared-session injections as local steering prompts.
21. Keep timing text visible when a long preview is clipped. Editing a row must not change its
    timing. Keep the timing distinction visible beside the inline editor.
22. Existing edit, delete, copy, and drag controls keep working. Reordering changes the order within
    each kind according to the resulting visible order; dragging across another kind does not
    convert a row or make full-response work eligible at a natural boundary.
23. **Send now** remains an explicit timing override for its selected row. Empty-input Enter still
    sends the first visible row if it is sendable, not the first automatic steering candidate.
    It must not skip a locked head. Existing locked-row restrictions still apply.
24. A waiting row owns its text and attachments. Dispatch must not steal attachments from a newer
    draft, overwrite the draft, or move keyboard focus away from the user's current input.
25. Clean completion must not auto-send an uncommitted inline edit. Preserve the existing
    edit-to-input fallback when its conditions apply. Natural-boundary steering waits for the
    edit to finish instead.
26. On error or cancellation, do not treat the event as a clean steering opportunity. Preserve the
    current queue recovery: restore at most the first visible eligible row into an empty input
    while viewing that conversation; otherwise leave the rows queued. Keep its attachments.
27. Keep queues isolated by conversation. Navigating away or switching panes must not redirect a
    waiting prompt into another conversation. Deleting its conversation removes its queue.
28. Do not deliver the same row twice if completion, attachment readiness, or a natural request
    arrives close together. A rejected dispatch must retain or restore the row rather than lose it.
29. Disabling the feature stops capture of new steering rows and hides its controls. Existing
    waiting rows keep their assigned timing until delivered or removed. An excluded conversation
    must not start using local steering merely because the synced setting is Steer.

## Intentional scope limits

- No redesign of the queue chip, panel grouping, or queue keyboard shortcuts.
- No cloud protocol, server, ACP, or third-party harness changes.
- No changes to long-running-command policies, skill execution, or slash-command expansion.
- This phase does not promote the flag to Preview or Stable.

## Assumptions proposed for approval

- Keep slash commands and skills unchanged in this phase; apply Steer to ordinary prompts.
- Keep mixed rows in their current visible order instead of adding grouped sections.
- At full-response completion, preserve visible-order dispatch rather than prioritize a kind
  when both kinds are eligible.
