use std::collections::HashMap;
use std::ffi::OsString;

use anyhow::Context as _;
use warp_cli::agent::Harness;
use warpui::{EntityId, SingletonEntity, ViewContext, ViewHandle};

use crate::ai::agent::conversation::{AIConversation, AIConversationId, RestoredChildHydration};
use crate::ai::ambient_agents::task::{AmbientAgentTask, ExecutionLocation};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::orchestration_event_streamer::agent_task_harness;
use crate::pane_group::PaneGroup;
use crate::terminal::TerminalView;

pub(crate) fn needs_restored_local_owner(task: &AmbientAgentTask) -> bool {
    task.execution_location == Some(ExecutionLocation::Local) && !task.is_third_party_harness()
}

impl PaneGroup {
    pub(crate) fn release_headless_child_seed(&mut self, ctx: &mut ViewContext<Self>) {
        self.headless_child_owners_ready = true;
        self.process_pending_parent_child_seeds(ctx);
    }
    pub(crate) fn prepare_restored_child_owner(
        &mut self,
        parent_conversation_id: AIConversationId,
        parent_terminal_id: EntityId,
        task: &AmbientAgentTask,
        env_vars: HashMap<OsString, OsString>,
        ctx: &mut ViewContext<Self>,
    ) -> (
        AIConversationId,
        anyhow::Result<Option<ViewHandle<TerminalView>>>,
    ) {
        let existing = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation_id_for_agent_id(&task.task_id.to_string());
        let child_id = existing
            .or_else(|| self.restored_child_owners.get(&task.task_id).copied())
            .unwrap_or_else(AIConversationId::new);
        self.restored_child_owners.insert(task.task_id, child_id);
        let result = (|| {
            if BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_id)
                .is_some_and(|child| {
                    child.restored_child_hydration() == Some(RestoredChildHydration::Error)
                })
            {
                return Ok(None);
            }
            if let Some(child) = BlocklistAIHistoryModel::as_ref(ctx).conversation(&child_id)
                && !child.is_remote_child()
                && !child.is_viewing_shared_session()
                && let Some(pane) = self.pane_id_for_owned_conversation(child_id, ctx)
                && self
                    .terminal_view_from_pane_id(pane, ctx)
                    .is_some_and(|terminal| terminal.id() != parent_terminal_id)
            {
                return Ok(None);
            }
            if self.child_agent_panes.contains_key(&child_id) {
                self.discard_child_agent_pane_for_conversation(child_id, ctx);
            }
            let mut child = AIConversation::new_restored_child(child_id);
            child.set_task_id(task.task_id);
            child.set_parent_conversation_id(parent_conversation_id);
            if let Some(parent_run) = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&parent_conversation_id)
                .and_then(AIConversation::run_id)
            {
                child.set_parent_agent_id(parent_run);
            }
            child.set_agent_name(task.display_name().to_owned());
            child.set_fallback_display_title(task.title.clone());
            child.set_orchestration_harness(Harness::Oz);
            child.set_last_event_sequence(task.last_event_sequence.unwrap_or(0));
            child.toggle_autoexecute_override();
            if let Some(token) = task
                .conversation_id()
                .filter(|token| !token.trim().is_empty())
            {
                child.set_server_conversation_token(token.to_owned());
            }
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.restore_conversations(parent_terminal_id, vec![child.clone()], ctx);
            });
            let parent_pane = self
                .find_pane_id_for_terminal_view(parent_terminal_id, ctx)
                .context("owner setup: parent pane missing")?;
            anyhow::ensure!(
                child.server_conversation_token().is_some(),
                "owner setup: missing conversation token"
            );
            self.create_hidden_child_agent_pane_with_env(child, parent_pane, env_vars, ctx);
            let pane = self
                .child_agent_panes
                .get(&child_id)
                .context("owner setup: hidden pane unavailable")?;
            self.terminal_view_from_pane_id(*pane, ctx)
                .map(Some)
                .context("owner setup: terminal/controller unavailable")
        })();
        (child_id, result)
    }

    pub(crate) fn finish_restored_child_owners(
        &mut self,
        parent_conversation_id: AIConversationId,
        parent_terminal_id: EntityId,
        children: &[AmbientAgentTask],
        ctx: &mut ViewContext<Self>,
    ) {
        for task in children {
            if needs_restored_local_owner(task) {
                continue;
            }
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                let child_id = history.ensure_remote_child_conversation(
                    parent_terminal_id,
                    parent_conversation_id,
                    task.task_id.to_string(),
                    task.task_id,
                    task.display_name().to_owned(),
                    task.title.clone(),
                    agent_task_harness(task),
                    ctx,
                );
                history.apply_remote_child_task_snapshot(parent_terminal_id, child_id, task, ctx);
            });
        }
        self.headless_child_owners_ready = true;
        self.process_pending_parent_child_seeds(ctx);
    }
}
