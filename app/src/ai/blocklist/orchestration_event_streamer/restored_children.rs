use anyhow::Context as _;
use warp_errors::report_error;
use warp_graphql::ai::AgentTaskState;
use warpui::{AppContext, ViewHandle};

use super::{
    AgentEventDriverConfig, BlocklistAIHistoryModel, DormantClaudeWakeConsumer, Harness,
    ModelContext, OrchestrationEventStreamer, ServerApiAgentEventSource, SingletonEntity,
    WakeConnectionState, run_agent_event_driver,
};
use crate::ai::agent::RenderableAIError;
use crate::ai::agent::api::convert_conversation::{
    RestorationMode, convert_conversation_data_to_ai_conversation,
};
use crate::ai::agent::conversation::{
    AIConversation, AIConversationId, ConversationStatus, RestoredChildHydration,
};
use crate::server::server_api::ai::TaskStatusUpdate;
use crate::terminal::TerminalView;
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};

impl OrchestrationEventStreamer {
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn register_restored_child(
        &mut self,
        conversation_id: AIConversationId,
        terminal: &ViewHandle<TerminalView>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.streams
            .entry(conversation_id)
            .or_default()
            .restored_owner = Some(terminal.downgrade());
        self.register_consumer(conversation_id, terminal.id(), ctx);
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn release_restored_child_subtree(
        &mut self,
        conversation_ids: &[AIConversationId],
        ctx: &mut ModelContext<Self>,
    ) {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            for conversation_id in conversation_ids {
                let surface = history.terminal_surface_id_for_conversation(conversation_id);
                let Some(child) = history.conversation_mut(conversation_id) else {
                    continue;
                };
                let was_ready = child.restored_child_execution_ready();
                child.release_restored_child_startup();
                if !was_ready
                    && child.restored_child_execution_ready()
                    && let Some(surface) = surface
                {
                    child.update_status_with_error(
                        child.status().clone(),
                        child.status_error().cloned(),
                        surface,
                        ctx,
                    );
                }
            }
        });
        for conversation_id in conversation_ids {
            self.reevaluate_eligibility(*conversation_id, ctx);
        }
    }
    pub(crate) fn restored_child_state(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<RestoredChildHydration> {
        BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(AIConversation::restored_child_hydration)
    }

    pub(super) fn reevaluate_restored_child(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        match self.restored_child_state(conversation_id, ctx) {
            None | Some(RestoredChildHydration::Ready) => false,
            Some(RestoredChildHydration::Unhydrated) => {
                self.teardown_sse(conversation_id, ctx);
                let stream = self.streams.entry(conversation_id).or_default();
                if stream.consumers.is_empty() {
                    self.teardown_dormant_claude_wake_listener(conversation_id);
                } else if stream.restored_owner.is_some() && stream.wake_connection.is_none() {
                    self.start_restored_child_wake_listener(conversation_id, ctx);
                }
                true
            }
            Some(RestoredChildHydration::Hydrating | RestoredChildHydration::Error) => {
                self.teardown_sse(conversation_id, ctx);
                self.teardown_dormant_claude_wake_listener(conversation_id);
                true
            }
        }
    }

    fn start_restored_child_wake_listener(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            return;
        };
        let Some(run_id) = conversation.run_id() else {
            return;
        };
        let cursor = conversation.last_event_sequence().unwrap_or(0);
        let stream = self.streams.entry(conversation_id).or_default();
        stream.event_cursor = cursor;
        stream.harness = Some(Harness::Oz);
        let generation = self.next_wake_generation;
        self.next_wake_generation += 1;
        let source = ServerApiAgentEventSource::new(self.server_api.clone());
        let task = ctx.spawn(
            async move {
                let config =
                    AgentEventDriverConfig::retry_forever_run_ids(vec![run_id.clone()], cursor);
                let mut consumer = DormantClaudeWakeConsumer::new(run_id);
                run_agent_event_driver(source, config, &mut consumer).await?;
                consumer
                    .wake_message
                    .context("child wake listener ended without a message")?;
                Ok::<_, anyhow::Error>(())
            },
            move |me, result, ctx| {
                if me
                    .streams
                    .get(&conversation_id)
                    .and_then(|stream| stream.wake_connection.as_ref())
                    .is_none_or(|connection| connection.generation != generation)
                {
                    return;
                }
                if let Some(stream) = me.streams.get_mut(&conversation_id) {
                    stream.wake_connection = None;
                }
                match result {
                    Ok(()) => {
                        me.hydrate_restored_child(conversation_id, ctx);
                    }
                    Err(error) => me.fail_restored_child(
                        conversation_id,
                        format!("wake listener: {error:#}"),
                        ctx,
                    ),
                }
            },
        );
        self.streams
            .entry(conversation_id)
            .or_default()
            .wake_connection = Some(WakeConnectionState { generation, task });
    }

    pub(crate) fn hydrate_restored_child(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        match self.restored_child_state(conversation_id, ctx) {
            None | Some(RestoredChildHydration::Ready) => return false,
            Some(RestoredChildHydration::Hydrating | RestoredChildHydration::Error) => return true,
            Some(RestoredChildHydration::Unhydrated) => {}
        }
        let Some(stub) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .cloned()
        else {
            return true;
        };
        let Some(token) = stub.server_conversation_token().cloned() else {
            self.fail_restored_child(conversation_id, "missing conversation token".into(), ctx);
            return true;
        };
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
            if let Some(conversation) = history.conversation_mut(&conversation_id) {
                conversation.set_restored_child_hydration(RestoredChildHydration::Hydrating);
            }
        });
        self.reevaluate_eligibility(conversation_id, ctx);
        let client = self.ai_client.clone();
        let generation = uuid::Uuid::new_v4();
        self.streams
            .entry(conversation_id)
            .or_default()
            .hydration_generation = Some(generation);
        ctx.spawn(
            async move {
                let (data, metadata) = client
                    .get_ai_conversation(token.clone())
                    .await
                    .context("fetch/decode transcript")?;
                anyhow::ensure!(
                    metadata.server_conversation_token == token,
                    "transcript token mismatch"
                );
                anyhow::ensure!(
                    metadata.harness == crate::ai::agent::conversation::AIAgentHarness::Oz,
                    "transcript harness is not Oz"
                );
                let mut hydrated = convert_conversation_data_to_ai_conversation(
                    conversation_id,
                    &data,
                    metadata,
                    RestorationMode::Continue,
                )
                .context("convert transcript")?;
                hydrated.set_task_id(stub.task_id().context("missing child run ID")?);
                hydrated.set_parent_agent_id(
                    stub.parent_agent_id()
                        .context("missing parent run ID")?
                        .to_string(),
                );
                hydrated.set_parent_conversation_id(
                    stub.parent_conversation_id()
                        .context("missing parent conversation")?,
                );
                hydrated.set_agent_name(stub.agent_name().unwrap_or("Agent").to_string());
                if let Some(title) = stub.title() {
                    hydrated.set_fallback_display_title(title);
                }
                if hydrated.autoexecute_override() != stub.autoexecute_override() {
                    hydrated.toggle_autoexecute_override();
                }
                hydrated.set_orchestration_harness(Harness::Oz);
                hydrated.set_last_event_sequence(stub.last_event_sequence().unwrap_or(0));
                hydrated.set_restored_child_hydration(RestoredChildHydration::Hydrating);
                Ok::<_, anyhow::Error>(hydrated)
            },
            move |me, result, ctx| {
                if me.restored_child_state(conversation_id, ctx)
                    != Some(RestoredChildHydration::Hydrating)
                    || me
                        .streams
                        .get(&conversation_id)
                        .and_then(|stream| stream.hydration_generation)
                        != Some(generation)
                {
                    return;
                }
                let result = result.and_then(|mut hydrated| {
                    let current = BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&conversation_id)
                        .context("install transcript: conversation missing")?;
                    hydrated.preserve_restored_child_startup(current);
                    let surface = BlocklistAIHistoryModel::as_ref(ctx)
                        .terminal_surface_id_for_conversation(&conversation_id)
                        .context("install transcript: missing terminal surface")?;
                    let terminal = me
                        .streams
                        .get(&conversation_id)
                        .and_then(|stream| stream.restored_owner.as_ref())
                        .and_then(|owner| owner.upgrade(ctx))
                        .filter(|owner| owner.id() == surface)
                        .context("install transcript: missing owner terminal")?;
                    terminal.update(ctx, |terminal, ctx| {
                        terminal.restore_conversation_after_view_creation(
                            RestoredAIConversation::new(hydrated),
                            true,
                            RestoreConversationEntryBehavior::PreserveAgentViewState,
                            ctx,
                        );
                    });
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        history
                            .conversation_mut(&conversation_id)
                            .context("install transcript: conversation missing")?
                            .set_restored_child_hydration(RestoredChildHydration::Ready);
                        history.update_conversation_status(
                            surface,
                            conversation_id,
                            ConversationStatus::Success,
                            ctx,
                        );
                        Ok::<_, anyhow::Error>(())
                    })?;
                    Ok(())
                });
                match result {
                    Ok(()) => me.reevaluate_eligibility(conversation_id, ctx),
                    Err(error) => {
                        me.fail_restored_child(conversation_id, format!("{error:#}"), ctx)
                    }
                }
            },
        );
        true
    }

    pub(crate) fn fail_restored_child(
        &mut self,
        conversation_id: AIConversationId,
        stage: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(task_id) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(AIConversation::task_id)
        else {
            report_error!("Cannot report restored child failure: missing run ID",
                extra: { "conversation_id" => %conversation_id });
            return;
        };
        let message = format!("Local child {task_id} restoration failed: {stage}");
        let surface = BlocklistAIHistoryModel::as_ref(ctx)
            .terminal_surface_id_for_conversation(&conversation_id);
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            if let Some(conversation) = history.conversation_mut(&conversation_id) {
                conversation.set_restored_child_hydration(RestoredChildHydration::Error);
                if let Some(surface) = surface {
                    conversation.update_status_with_error(
                        ConversationStatus::Error,
                        Some(RenderableAIError::other(message.clone(), false)),
                        surface,
                        ctx,
                    );
                }
            }
        });
        self.reevaluate_eligibility(conversation_id, ctx);
        let client = self.ai_client.clone();
        ctx.spawn(
            async move {
                client
                    .update_agent_task(
                        task_id,
                        Some(AgentTaskState::Error),
                        None,
                        None,
                        Some(TaskStatusUpdate::message(message)),
                        None,
                        None,
                    )
                    .await
            },
            move |_, result, _| {
                if let Err(error) = result {
                    report_error!(error.context("Failed to report restored child Error"),
                        extra: { "child_run_id" => %task_id });
                }
            },
        );
    }
}
