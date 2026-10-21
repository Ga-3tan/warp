# APP-6429 — Interrupt or steer local agent prompts

## Summary
Add an Interrupt / Steer setting without changing full-response queueing.
Issue: [APP-6429](https://linear.app/warpdotdev/issue/APP-6429).

**For review:** the setting name and row labels, mixed-row ordering at full completion, and whether
ordinary prompts are the right initial scope. Implementation waits for approval.

## Behavior
1. **Separate setting.** Add **Unqueued prompt behavior: Interrupt / Steer** beside the existing
   prompt-submission settings. Default to Interrupt. Persist and sync it with the user's settings;
   changes affect future submissions only. Make it searchable and available in the Command Palette.

2. **Queueing still wins.** The existing default queue setting, auto-queue chip, and `/queue` still
   wait for the entire response. Keep the chip and shortcut unchanged; do not add a steering chip.
   Long-running-command, compaction, and setup protections retain precedence.

3. **Interrupt versus Steer.** With queueing and safety holds inactive:
   - Interrupt sends immediately through today's interrupt path.
   - Steer waits for the next natural agent request, such as a tool-result continuation, without
     cancelling current work. Deliver one prompt per request, never merge prompts.
   - If no continuation occurs, deliver on clean full-response completion. Idle submissions send
     normally. Error or cancellation must not trigger automatic delivery.

4. **FIFO within each kind.** A steering correction can pass older full-response rows, but cannot
   pass an earlier steering row that is being edited or is not ready.
   - Example: `/queue write tests`, then “do not modify X” in Steer mode. The correction goes at the
     next natural request; “write tests” waits for the full response.
   - **Proposed detail:** when the full response finishes and both kinds are eligible, dispatch the
     first visible row. Keep mixed rows in their existing panel order, without grouping.

5. **Same panel, explicit timing.** Reuse the current italic suffix treatment:
   **(next turn)** for steering and **(after response)** for full-response prompts. Keep the existing
   command-finish suffix and shell-command styling. Timing stays visible with long text and editing.

6. **Existing controls and recovery.** Keep editing, deleting, copying, reordering, and attachment
   ownership. Reordering does not change a row's timing. Send now overrides timing; empty-input
   Enter still targets the first visible row without skipping a locked head. Preserve drafts,
   conversation isolation, and existing error/edit recovery. Never duplicate or lose a row on
   dispatch failure. Pending rows retain their timing if the setting or flag changes.

7. **Initial scope and rollout.** Native local desktop Warp Agent only, behind a dogfood/staging-only
   flag; no Preview, Stable, or OSS rollout.
   **Proposed detail:** ordinary prompts only. Keep slash/skill handling, shell commands, cloud,
   shared viewers, third-party harnesses, CLI rich input, and the TUI unchanged.

Figma: none provided; reuse current controls, as requested. Computer-use verification is opted in.
