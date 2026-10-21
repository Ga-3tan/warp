use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{Context as _, bail};

use super::{AmbientAgentTask, AmbientAgentTaskId, ExecutionLocation};
use crate::server::server_api::ai::{AIClient, RunSortBy, RunSortOrder, TaskListFilter};

/// Returns all direct children, or an error without exposing a partial traversal.
pub async fn discover_direct_children(
    ai_client: &dyn AIClient,
    parent_task_id: AmbientAgentTaskId,
) -> anyhow::Result<Vec<AmbientAgentTask>> {
    Ok(discover_children_with_harness(ai_client, parent_task_id)
        .await?
        .into_iter()
        .map(|(task, _)| task)
        .collect())
}

pub(crate) fn needs_restored_local_owner(task: &AmbientAgentTask) -> bool {
    task.execution_location == Some(ExecutionLocation::Local) && !task.is_third_party_harness()
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) struct ChildRunBatch {
    pub parent_task_id: AmbientAgentTaskId,
    pub children: Vec<AmbientAgentTask>,
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) async fn discover_local_child_subtree(
    ai_client: &dyn AIClient,
    root_task_id: AmbientAgentTaskId,
) -> anyhow::Result<Vec<ChildRunBatch>> {
    let mut parents = VecDeque::from([root_task_id]);
    let mut identities = HashMap::new();
    let mut batches = Vec::new();
    while let Some(parent_task_id) = parents.pop_front() {
        let rows = discover_children_with_harness(ai_client, parent_task_id).await?;
        let mut children = Vec::new();
        for (child, harness) in rows {
            anyhow::ensure!(
                child.task_id != root_task_id,
                "root run returned as a descendant"
            );
            let identity = (
                child.parent_run_id.clone(),
                child.execution_location,
                harness,
            );
            if let Some(prior) = identities.get(&child.task_id) {
                anyhow::ensure!(
                    prior == &identity,
                    "conflicting subtree child {}",
                    child.task_id
                );
                continue;
            }
            identities.insert(child.task_id, identity);
            if needs_restored_local_owner(&child) {
                parents.push_back(child.task_id);
            }
            children.push(child);
        }
        batches.push(ChildRunBatch {
            parent_task_id,
            children,
        });
    }
    Ok(batches)
}

async fn discover_children_with_harness(
    ai_client: &dyn AIClient,
    parent_task_id: AmbientAgentTaskId,
) -> anyhow::Result<Vec<(AmbientAgentTask, Option<String>)>> {
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
            return Ok(children.into_iter().zip(harness_names).collect());
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
