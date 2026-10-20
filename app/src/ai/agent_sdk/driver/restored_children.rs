use super::*;
use crate::ai::ambient_agents::discovery::discover_direct_children;
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::pane_group::child_agent::restored_owners::needs_restored_local_owner;
use crate::server::retry_strategies::is_transient_http_error;

impl AgentDriver {
    pub(super) async fn restore_local_child_owners(
        foreground: &ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        let Some((parent_id, parent_conversation_id, terminal_id, group, client, env, working_dir)) =
            foreground
                .spawn(|me, ctx| {
                    if !FeatureFlag::RestoreLocalChildOwners.is_enabled() {
                        return None;
                    }
                    let parent_id = me.task_id?;
                    let conversation_id = me.restored_conversation_id?;
                    let terminal = me.terminal_driver.as_ref(ctx);
                    Some((
                        parent_id,
                        conversation_id,
                        terminal.terminal_view().id(),
                        terminal.pane_group().cloned(),
                        ServerApiProvider::as_ref(ctx).get_ai_client(),
                        me.resolved_env_vars.clone(),
                        me.harness_working_dir.clone(),
                    ))
                })
                .await?
        else {
            foreground
                .spawn(|me, ctx| {
                    if let Some(group) = me.terminal_driver.as_ref(ctx).pane_group().cloned() {
                        group.update(ctx, |group, ctx| group.release_headless_child_seed(ctx));
                    }
                })
                .await?;
            return Ok(());
        };
        let group = group.ok_or(AgentDriverError::TerminalUnavailable)?;
        let mut attempt = 0;
        let children = loop {
            match discover_direct_children(client.as_ref(), parent_id).await {
                Ok(children) => break children,
                Err(error) if is_transient_http_error(&error) && attempt < 4 => {
                    attempt += 1;
                    log::warn!(
                        "Child discovery for {parent_id} failed (attempt {attempt}): {error:#}"
                    );
                    Timer::after(Duration::from_secs(attempt)).await;
                }
                Err(error) => {
                    return Err(AgentDriverError::EnvironmentSetupFailed(format!(
                        "Restoring children of {parent_id}: {error:#}"
                    )));
                }
            }
        };
        for task in children
            .iter()
            .filter(|task| needs_restored_local_owner(task))
        {
            let task = task.clone();
            let group = group.clone();
            let env = env.clone();
            let (child_id, terminal) = foreground
                .spawn(move |_, ctx| {
                    let mut env_vars = (*env).clone();
                    env_vars.extend(task_env_vars(
                        Some(&task.task_id),
                        Some(&parent_id.to_string()),
                        Harness::Oz,
                    ));
                    group.update(ctx, |group, ctx| {
                        group.prepare_restored_child_owner(
                            parent_conversation_id,
                            terminal_id,
                            &task,
                            env_vars,
                            ctx,
                        )
                    })
                })
                .await?;
            let result = match terminal {
                Ok(None) => continue,
                Ok(Some(terminal)) => {
                    let terminal_for_bootstrap = terminal.clone();
                    let driver = foreground
                        .spawn(move |_, ctx| {
                            terminal::TerminalDriver::create_from_existing_view(
                                terminal_for_bootstrap,
                                ctx,
                            )
                        })
                        .await?;
                    let bootstrap = foreground
                        .spawn({
                            let driver = driver.clone();
                            move |_, ctx| {
                                driver
                                    .update(ctx, |driver, _| driver.wait_for_session_bootstrapped())
                            }
                        })
                        .await?;
                    match bootstrap.await {
                        Ok(()) => {
                            let directory = working_dir.to_string_lossy().into_owned();
                            let cd = foreground
                                .spawn(move |_, ctx| {
                                    driver.update(ctx, |driver, ctx| {
                                        driver.cd_silent(&directory, ctx)
                                    })
                                })
                                .await?;
                            match cd.await {
                                Ok(output)
                                    if output
                                        .exit_code
                                        .is_some_and(|code| code.was_successful()) =>
                                {
                                    Ok(terminal)
                                }
                                Ok(output) => Err(anyhow!(
                                    "owner shell directory setup failed: {:?}",
                                    output.exit_code
                                )),
                                Err(error) => Err(anyhow!(error)),
                            }
                        }
                        Err(error) => Err(anyhow!("owner shell bootstrap: {error}")),
                    }
                }
                Err(error) => Err(error),
            };
            foreground
                .spawn(move |_, ctx| {
                    OrchestrationEventStreamer::handle(ctx).update(
                        ctx,
                        |streamer, ctx| match result {
                            Ok(terminal) => {
                                streamer.register_restored_child_owner(child_id, &terminal, ctx)
                            }
                            Err(error) => {
                                streamer.fail_restored_child(child_id, format!("{error:#}"), ctx)
                            }
                        },
                    );
                })
                .await?;
        }
        foreground
            .spawn(move |_, ctx| {
                group.update(ctx, |group, ctx| {
                    group.finish_restored_child_owners(
                        parent_conversation_id,
                        terminal_id,
                        &children,
                        ctx,
                    )
                });
            })
            .await?;
        Ok(())
    }
}
