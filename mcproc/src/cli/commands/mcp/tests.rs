use super::test_support::McpTestHarness;
use super::tools::{GrepTool, LogsTool, PsTool, RestartTool, StartTool, StatusTool, StopTool};
use mcp_rs::{Error as McpError, ToolHandler};
use serde_json::{json, Value};

const PROJECT: &str = "mcp-tool-tests";

async fn start_process(harness: &McpTestHarness, name: &str, command: &str) -> Value {
    StartTool::new(harness.client.clone())
        .handle(
            Some(json!({
                "name": name,
                "cmd": command,
                "project": PROJECT,
            })),
            McpTestHarness::context(),
        )
        .await
        .unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn all_tools_publish_nonempty_object_schemas_with_declared_required_fields() {
    let harness = McpTestHarness::new().await;
    let tools: Vec<(Box<dyn ToolHandler>, &[&str])> = vec![
        (Box::new(StartTool::new(harness.client.clone())), &["name"]),
        (Box::new(StopTool::new(harness.client.clone())), &["name"]),
        (
            Box::new(RestartTool::new(harness.client.clone())),
            &["name"],
        ),
        (Box::new(PsTool::new(harness.client.clone())), &[]),
        (Box::new(StatusTool::new(harness.client.clone())), &["name"]),
        (Box::new(LogsTool::new(harness.client.clone())), &["name"]),
        (
            Box::new(GrepTool::new(harness.client.clone())),
            &["pattern", "name"],
        ),
    ];

    for (tool, expected_required) in tools {
        let info = tool.tool_info();
        assert!(!info.name.trim().is_empty());
        assert!(!info.description.trim().is_empty(), "{}", info.name);
        assert_eq!(info.input_schema["type"], "object", "{}", info.name);

        let properties = info.input_schema["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{} schema must declare properties", info.name));
        if !expected_required.is_empty() {
            let required = info.input_schema["required"]
                .as_array()
                .unwrap_or_else(|| panic!("{} schema must declare required", info.name));
            for field in expected_required {
                assert!(
                    required.iter().any(|value| value == field),
                    "{} must require {field}",
                    info.name
                );
                assert!(
                    properties.contains_key(*field),
                    "{} required field {field} must exist in properties",
                    info.name
                );
            }
        }
    }

    harness.cleanup().await;
}

#[cfg(unix)]
#[tokio::test]
async fn start_returns_process_identity_and_rejects_missing_command() {
    let harness = McpTestHarness::new().await;
    let started = start_process(&harness, "start-target", "echo hello-from-start; sleep 30").await;

    assert_eq!(started["name"], "start-target");
    assert!(started["status"].as_str().is_some());
    assert!(started["pid"].as_u64().is_some_and(|pid| pid > 0));

    let error = StartTool::new(harness.client.clone())
        .handle(
            Some(json!({ "name": "invalid-start", "project": PROJECT })),
            McpTestHarness::context(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::InvalidParams(_)));
    assert!(error.to_string().contains("either 'cmd' or 'args'"));

    let stopped = StopTool::new(harness.client.clone())
        .handle(
            Some(json!({ "name": "start-target", "project": PROJECT })),
            McpTestHarness::context(),
        )
        .await
        .unwrap();
    assert_eq!(stopped["success"], true);
    harness.cleanup().await;
}

#[cfg(unix)]
#[tokio::test]
async fn stop_reports_success_for_existing_process_and_false_for_missing_process() {
    let harness = McpTestHarness::new().await;
    start_process(&harness, "stop-target", "sleep 30").await;
    let tool = StopTool::new(harness.client.clone());

    let stopped = tool
        .handle(
            Some(json!({ "name": "stop-target", "project": PROJECT })),
            McpTestHarness::context(),
        )
        .await
        .unwrap();
    assert_eq!(stopped["success"], true);

    let missing = tool
        .handle(
            Some(json!({ "name": "not-present", "project": PROJECT })),
            McpTestHarness::context(),
        )
        .await
        .unwrap();
    assert_eq!(missing["success"], false);
    assert!(missing["message"].as_str().unwrap().contains("not found"));
    harness.cleanup().await;
}

#[cfg(unix)]
#[tokio::test]
async fn restart_replaces_the_running_process_pid() {
    let harness = McpTestHarness::new().await;
    let started = start_process(&harness, "restart-target", "sleep 30").await;
    let old_pid = started["pid"].as_u64().unwrap();

    let restarted = RestartTool::new(harness.client.clone())
        .handle(
            Some(json!({ "name": "restart-target", "project": PROJECT })),
            McpTestHarness::context(),
        )
        .await
        .unwrap();
    assert_eq!(restarted["name"], "restart-target");
    assert!(restarted["status"].as_str().is_some());
    assert_ne!(restarted["pid"].as_u64().unwrap(), old_pid);
    harness.cleanup().await;
}

#[cfg(unix)]
#[tokio::test]
async fn ps_lists_the_started_process() {
    let harness = McpTestHarness::new().await;
    start_process(&harness, "ps-target", "sleep 30").await;

    let response = PsTool::new(harness.client.clone())
        .handle(Some(json!({})), McpTestHarness::context())
        .await
        .unwrap();
    let processes = response["processes"].as_array().unwrap();
    assert!(processes
        .iter()
        .any(|process| { process["name"] == "ps-target" && process["project"] == PROJECT }));
    harness.cleanup().await;
}

#[cfg(unix)]
#[tokio::test]
async fn status_returns_the_started_process_name_and_state() {
    let harness = McpTestHarness::new().await;
    start_process(&harness, "status-target", "sleep 30").await;

    let response = StatusTool::new(harness.client.clone())
        .handle(
            Some(json!({ "name": "status-target", "project": PROJECT })),
            McpTestHarness::context(),
        )
        .await
        .unwrap();
    assert_eq!(response["name"], "status-target");
    assert!(response["status"].as_str().is_some());
    harness.cleanup().await;
}

#[cfg(unix)]
#[tokio::test]
async fn logs_and_grep_return_context_and_strip_ansi_sequences() {
    let harness = McpTestHarness::new().await;
    let started = start_process(
        &harness,
        "log-target",
        "printf 'before-line\\n\\033[31mneedle-red\\033[0m\\nafter-line\\n'; sleep 30",
    )
    .await;
    let log_file = started["log_file"].as_str().unwrap();
    harness
        .wait_for_log(log_file, &["before-line", "needle-red", "after-line"])
        .await;

    let logs = LogsTool::new(harness.client.clone())
        .handle(
            Some(json!({ "name": "log-target", "tail": 10, "project": PROJECT })),
            McpTestHarness::context(),
        )
        .await
        .unwrap();
    let log_text = serde_json::to_string(&logs).unwrap();
    assert!(log_text.contains("before-line"));
    assert!(log_text.contains("needle-red"));
    assert!(log_text.contains("after-line"));
    assert!(!log_text.contains("\\u001b"));
    assert!(!log_text.contains("[31m"));

    let grep = GrepTool::new(harness.client.clone())
        .handle(
            Some(json!({
                "name": "log-target",
                "pattern": "needle-red",
                "context": 1,
                "project": PROJECT,
            })),
            McpTestHarness::context(),
        )
        .await
        .unwrap();
    let grep_text = serde_json::to_string(&grep).unwrap();
    assert_eq!(grep["total_matches"], 1);
    assert!(grep_text.contains("before-line"));
    assert!(grep_text.contains("needle-red"));
    assert!(grep_text.contains("after-line"));
    assert!(!grep_text.contains("\\u001b"));
    assert!(!grep_text.contains("[31m"));
    harness.cleanup().await;
}
