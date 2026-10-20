use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, bail};

use super::{AmbientAgentTask, AmbientAgentTaskId};
use crate::server::server_api::ai::{AIClient, RunSortBy, RunSortOrder, TaskListFilter};

/// Returns all direct children, or an error without exposing a partial traversal.
pub async fn discover_direct_children(
    ai_client: &dyn AIClient,
    parent_task_id: AmbientAgentTaskId,
) -> anyhow::Result<Vec<AmbientAgentTask>> {
    let parent_id = parent_task_id.to_string();
    let mut cursor = None;
    let mut cursors = HashSet::new();
    let mut children: Vec<AmbientAgentTask> = Vec::new();
    let mut indices = HashMap::new();
    let mut harness_names = Vec::new();
    loop {
        let page = ai_client
            .list_agent_runs_page(
                100,
                TaskListFilter {
                    ancestor_run_id: Some(parent_id.clone()),
                    sort_by: Some(RunSortBy::CreatedAt),
                    sort_order: Some(RunSortOrder::Asc),
                    cursor,
                    ..TaskListFilter::default()
                },
            )
            .await
            .context("discovering direct child runs")?;
        for (row, child) in page.runs.into_iter().enumerate() {
            if child.task_id == parent_task_id {
                continue;
            }
            if child.execution_location.is_none() {
                bail!("child {} has no execution location", child.task_id);
            }
            if child.parent_run_id.as_deref() != Some(parent_id.as_str()) {
                bail!("child {} has an unexpected parent", child.task_id);
            }
            let harness_name = page.harness_names.get(row).cloned().unwrap_or_else(|| {
                child
                    .agent_config_snapshot
                    .as_ref()
                    .and_then(|config| config.harness.as_ref())
                    .map(|harness| harness.harness_type.config_name().to_owned())
            });
            if let Some(&index) = indices.get(&child.task_id) {
                let prior: &AmbientAgentTask = &children[index];
                if prior.parent_run_id != child.parent_run_id
                    || prior.execution_location != child.execution_location
                    || harness_names[index] != harness_name
                {
                    bail!("conflicting duplicate child {}", child.task_id);
                }
            } else {
                indices.insert(child.task_id, children.len());
                children.push(child);
                harness_names.push(harness_name);
            }
        }
        if !page.page_info.has_next_page {
            return Ok(children);
        }
        let next = page
            .page_info
            .next_cursor
            .filter(|cursor| !cursor.trim().is_empty())
            .context("child discovery page is missing its next cursor")?;
        if !cursors.insert(next.clone()) {
            bail!("child discovery repeated a page cursor");
        }
        cursor = Some(next);
    }
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
