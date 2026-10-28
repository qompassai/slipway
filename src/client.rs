//! D-121 client half: slipway as a modern-era MCP CLIENT to
//! the D-118 in-app servers. Discover-first (rmcp
//! `ClientLifecycleMode::Discover` — `initialize` is never
//! sent), per-request `_meta` carried by the SDK, and a scripted
//! QA scenario per app whose assertions are the app's own QA
//! contracts (light-show: the D-118 playtest sequence and hash
//! determinism; ONTrack: the 1°-grid fixture, ~333568 m over
//! three legs, byte-identical across two calls).

use serde_json::{Value, json};

use rmcp::model::{CallToolRequestParams, CallToolResult, ProtocolVersion};
use rmcp::service::{ClientLifecycleMode, ClientServiceExt};
use rmcp::transport::TokioChildProcess;

use crate::config::Project;
use crate::model::Failure;

fn result_payload(result: &CallToolResult) -> Value {
    if let Some(structured) = &result.structured_content {
        return structured.clone();
    }
    for block in &result.content {
        if let Some(text) = block.as_text() {
            if let Ok(parsed) = serde_json::from_str::<Value>(&text.text) {
                return parsed;
            }
            return json!({"text": text.text});
        }
    }
    json!(null)
}

struct Step {
    name: String,
    ok: bool,
    detail: String,
}

/// Drive one project's in-app server through its QA scenario.
/// Returns the full transcript (also receipted by the caller).
pub async fn drive_app(project: &Project) -> Result<Value, Failure> {
    if project.app_server_command.is_empty() {
        return Err(Failure::domain(
            "app_server_unconfigured",
            format!(
                "project {} has no in-app server command configured",
                project.id
            ),
        ));
    }
    let mut command = tokio::process::Command::new(&project.app_server_command[0]);
    command.args(&project.app_server_command[1..]);
    let transport = TokioChildProcess::new(command).map_err(|e| {
        Failure::domain(
            "app_server_spawn_failed",
            format!("cannot spawn app server: {e}"),
        )
    })?;
    let service = ()
        .serve_with_lifecycle(
            transport,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await
        .map_err(|e| {
            Failure::domain(
                "app_server_connect_failed",
                format!("discover-first connect failed: {e}"),
            )
        })?;
    let peer = service.peer().clone();
    let mut steps: Vec<Step> = Vec::new();
    let mut calls: Vec<Value> = Vec::new();

    let info = service.peer_info();
    let versions = info
        .as_ref()
        .map(|i| format!("{:?}", i.protocol_version))
        .unwrap_or_else(|| "unknown".to_string());
    steps.push(Step {
        name: "discover_modern_only".to_string(),
        ok: versions.contains("2026-07-28"),
        detail: format!("peer protocol version: {versions}"),
    });

    let tools = peer
        .list_all_tools()
        .await
        .map_err(|e| Failure::domain("app_server_failed", format!("tools/list failed: {e}")))?;
    let tool_names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    steps.push(Step {
        name: "tools_list".to_string(),
        ok: !tool_names.is_empty(),
        detail: format!("{} tools: {}", tool_names.len(), tool_names.join(", ")),
    });

    let call = |name: &'static str, args: Value| {
        let peer = peer.clone();
        async move {
            let mut params = CallToolRequestParams::new(name);
            if let Some(object) = args.as_object() {
                params = params.with_arguments(object.clone());
            }
            peer.call_tool(params).await
        }
    };

    match project.id.as_str() {
        "light-show" => {
            let levels = call("list_levels", json!({})).await.map_err(|e| {
                Failure::domain("app_server_failed", format!("list_levels failed: {e}"))
            })?;
            let payload = result_payload(&levels);
            let count = payload["levels"].as_array().map(|a| a.len()).unwrap_or(0);
            calls.push(json!({"tool": "list_levels", "result": payload}));
            steps.push(Step {
                name: "list_levels".to_string(),
                ok: count >= 2,
                detail: format!("{count} levels listed"),
            });
            let session = "slipway-qa";
            let loaded = call("load_level", json!({"level_index": 0, "session": session}))
                .await
                .map_err(|e| {
                    Failure::domain("app_server_failed", format!("load_level failed: {e}"))
                })?;
            calls.push(json!({"tool": "load_level", "result": result_payload(&loaded)}));
            let moved = call("apply_move", json!({"choice_index": 0, "session": session}))
                .await
                .map_err(|e| {
                    Failure::domain("app_server_failed", format!("apply_move failed: {e}"))
                })?;
            let moved_payload = result_payload(&moved);
            calls.push(json!({"tool": "apply_move", "result": moved_payload}));
            let sim = call("step_sim", json!({"session": session}))
                .await
                .map_err(|e| {
                    Failure::domain("app_server_failed", format!("step_sim failed: {e}"))
                })?;
            let sim_payload = result_payload(&sim);
            calls.push(json!({"tool": "step_sim", "result": sim_payload.clone()}));
            steps.push(Step {
                name: "playtest_step_sim".to_string(),
                ok: sim_payload.get("received_dbm").is_some()
                    || sim_payload.get("evaluation").is_some(),
                detail: "step_sim returned an evaluation".to_string(),
            });
            let run1 = call("run_playtest", json!({"level_index": 0, "moves": [0]}))
                .await
                .map_err(|e| {
                    Failure::domain("app_server_failed", format!("run_playtest failed: {e}"))
                })?;
            let run2 = call("run_playtest", json!({"level_index": 0, "moves": [0]}))
                .await
                .map_err(|e| {
                    Failure::domain("app_server_failed", format!("run_playtest #2 failed: {e}"))
                })?;
            let p1 = result_payload(&run1);
            let p2 = result_payload(&run2);
            calls.push(json!({"tool": "run_playtest", "result": p1}));
            steps.push(Step {
                name: "playtest_deterministic".to_string(),
                ok: p1 == p2,
                detail: "two run_playtest calls byte-identical".to_string(),
            });
        }
        "ontrack" => {
            let status = call("app_status", json!({})).await.map_err(|e| {
                Failure::domain("app_server_failed", format!("app_status failed: {e}"))
            })?;
            let status_payload = result_payload(&status);
            calls.push(json!({"tool": "app_status", "result": status_payload.clone()}));
            steps.push(Step {
                name: "app_status".to_string(),
                ok: status_payload.to_string().contains("2026-07-28"),
                detail: "app_status reports the modern protocol version".to_string(),
            });
            let stops = json!([
                {"address": "Depot", "lat": 0.0, "lng": 0.0},
                {"address": "North", "lat": 1.0, "lng": 0.0},
                {"address": "East", "lat": 0.0, "lng": 1.0},
                {"address": "Corner", "lat": 1.0, "lng": 1.0}
            ]);
            let args = json!({"stops": stops, "backend": "haversine"});
            let r1 = call("optimize_route", args.clone()).await.map_err(|e| {
                Failure::domain("app_server_failed", format!("optimize_route failed: {e}"))
            })?;
            let r2 = call("optimize_route", args).await.map_err(|e| {
                Failure::domain(
                    "app_server_failed",
                    format!("optimize_route #2 failed: {e}"),
                )
            })?;
            let p1 = result_payload(&r1);
            let p2 = result_payload(&r2);
            calls.push(json!({"tool": "optimize_route", "result": p1.clone()}));
            steps.push(Step {
                name: "optimize_deterministic".to_string(),
                ok: p1 == p2,
                detail: "two optimize_route calls byte-identical".to_string(),
            });
            let cost = p1["total_cost"].as_f64().unwrap_or(f64::NAN);
            steps.push(Step {
                name: "optimize_cost_fixture".to_string(),
                ok: (cost - 333_567.84).abs() < 100.0,
                detail: format!("total_cost {cost} m over the 1-degree grid (expect ≈333568)"),
            });
            let ordered = p1["ordered_indices"].clone();
            steps.push(Step {
                name: "optimize_visits_all".to_string(),
                ok: ordered
                    .as_array()
                    .is_some_and(|a| a.len() == 4 && a[0] == json!(0)),
                detail: format!("ordered_indices {ordered}"),
            });
        }
        other => {
            return Err(Failure::domain(
                "app_scenario_missing",
                format!("no QA scenario is defined for project {other}"),
            ));
        }
    }

    let _ = service.cancel().await;
    let passed = steps.iter().filter(|s| s.ok).count();
    Ok(json!({
        "project": project.id,
        "server_command": project.app_server_command,
        "tools": tool_names,
        "steps": steps.iter().map(|s| json!({"step": s.name, "ok": s.ok, "detail": s.detail})).collect::<Vec<_>>(),
        "steps_passed": passed,
        "steps_total": steps.len(),
        "all_passed": passed == steps.len(),
        "calls": calls,
    }))
}
