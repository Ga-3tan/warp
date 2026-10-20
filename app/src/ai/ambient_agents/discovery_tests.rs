use mockall::Sequence;
use serde_json::{Value, json};

use super::*;
use crate::pane_group::child_agent::restored_owners::needs_restored_local_owner;
use crate::server::server_api::ai::{AgentRunsPage, MockAIClient};

fn run(id: u128) -> Value {
    json!({
        "task_id": uuid::Uuid::from_u128(if id == 0 { 9999 } else { id }).to_string(),
        "parent_run_id": uuid::Uuid::from_u128(9999).to_string(),
        "title": "child",
        "state": "SUCCEEDED",
        "prompt": "work",
        "created_at": "2026-10-01T00:00:00Z",
        "updated_at": "2026-10-01T00:00:00Z",
        "execution_location": if id.is_multiple_of(2) { "LOCAL" } else { "REMOTE" },
        "is_sandbox_running": false
    })
}

fn page(runs: Vec<Value>, has_next_page: bool, cursor: Option<&str>) -> AgentRunsPage {
    serde_json::from_value(json!({
        "runs": runs,
        "page_info": { "has_next_page": has_next_page, "next_cursor": cursor }
    }))
    .unwrap()
}

#[test]
fn restored_local_child_discovery_pages() {
    let mut client = MockAIClient::new();
    let mut sequence = Sequence::new();
    client
        .expect_list_agent_runs_page()
        .withf(|limit, filter| {
            *limit == 100
                && filter.ancestor_run_id.as_deref()
                    == Some(uuid::Uuid::from_u128(9999).to_string().as_str())
                && filter.execution_location.is_none()
                && filter.sort_by == Some(RunSortBy::CreatedAt)
                && filter.sort_order == Some(RunSortOrder::Asc)
                && filter.cursor.is_none()
        })
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_, _| Ok(page((0..100).map(run).collect(), true, Some("page-2"))));
    client
        .expect_list_agent_runs_page()
        .withf(|_, filter| filter.cursor.as_deref() == Some("page-2"))
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_, _| Ok(page(vec![run(99), run(100), run(101)], false, None)));
    let children = futures::executor::block_on(discover_direct_children(
        &client,
        uuid::Uuid::from_u128(9999).to_string().parse().unwrap(),
    ))
    .unwrap();
    assert_eq!(children.len(), 101);
    assert_eq!(
        children.first().unwrap().task_id.to_string(),
        uuid::Uuid::from_u128(1).to_string()
    );
    assert_eq!(
        children.last().unwrap().task_id.to_string(),
        uuid::Uuid::from_u128(101).to_string()
    );
}

#[test]
fn restored_local_child_discovery_rejects_incomplete_traversal() {
    let cases = [
        (vec![run(1)], true, None),
        (vec![run(1)], true, Some(" ")),
        (vec![run(1)], true, Some("page-2")),
        (
            vec![{
                let mut row = run(1);
                row["execution_location"] = Value::Null;
                row
            }],
            false,
            None,
        ),
        (
            vec![{
                let mut row = run(1);
                row["parent_run_id"] = json!(uuid::Uuid::from_u128(42));
                row
            }],
            false,
            None,
        ),
        (
            vec![run(1), {
                let mut row = run(1);
                row["execution_location"] = json!("LOCAL");
                row
            }],
            false,
            None,
        ),
        (
            vec![run(1), {
                let mut row = run(1);
                row["agent_config_snapshot"] = json!({"harness": {"type": "claude"}});
                row
            }],
            false,
            None,
        ),
        (
            ["future-a", "future-b"]
                .into_iter()
                .map(|harness| {
                    let mut row = run(1);
                    row["agent_config"] = json!({"harness": {"type": harness}});
                    row
                })
                .collect(),
            false,
            None,
        ),
    ];
    for (runs, has_next, cursor) in cases {
        let mut client = MockAIClient::new();
        let mut first = Some(page(runs, has_next, cursor));
        client.expect_list_agent_runs_page().returning(move |_, _| {
            Ok(first
                .take()
                .unwrap_or_else(|| page(vec![run(2)], true, Some("page-2"))))
        });
        assert!(
            futures::executor::block_on(discover_direct_children(
                &client,
                uuid::Uuid::from_u128(9999).to_string().parse().unwrap(),
            ))
            .is_err()
        );
    }
}

#[test]
fn restored_local_child_strict_page_decoder() {
    for response in [
        json!({"runs": [run(1), {"task_id": "invalid"}], "page_info": {"has_next_page": false}}),
        json!({"runs": [run(1)]}),
        json!({"runs": [], "page_info": {"next_cursor": "next"}}),
    ] {
        assert!(serde_json::from_value::<AgentRunsPage>(response).is_err());
    }
}

#[test]
fn restored_local_child_classification() {
    for (location, harness, expected) in [
        ("LOCAL", None, true),
        ("LOCAL", Some("oz"), true),
        ("REMOTE", None, false),
        ("LOCAL", Some("claude"), false),
        ("LOCAL", Some("unknown-future-harness"), false),
    ] {
        let mut row = run(2);
        row["execution_location"] = json!(location);
        if let Some(harness) = harness {
            row["agent_config_snapshot"] = json!({"harness": {"type": harness}});
        }
        let task: AmbientAgentTask = serde_json::from_value(row).unwrap();
        assert_eq!(
            needs_restored_local_owner(&task),
            expected,
            "{location} {harness:?}"
        );
    }
}
