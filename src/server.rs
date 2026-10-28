//! rmcp server surface (protocol 2026-07-28 era only).
//!
//! Same shape as the fleet template (WS1/WS2-proven): handler
//! pinned to the single modern version, tools built as raw
//! schemas with annotations from birth, argument shape failures
//! answering -32602 and domain refusals as typed `is_error`
//! results. Synchronous tools dispatch on `spawn_blocking`;
//! `slipway_drive_app` (the D-121 client half) is awaited inline
//! because it is itself async.

use std::borrow::Cow;
use std::sync::Arc;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities,
    Tool, ToolAnnotations,
};
use rmcp::{ErrorData as McpError, RoleServer, ServiceExt};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::client;
use crate::config::Config;
use crate::model::Failure;
use crate::receipt;
use crate::tools;

fn schema(
    properties: &[(&str, &str, &str)],
    required: &[&str],
    extra: Value,
) -> Arc<Map<String, Value>> {
    let mut props = Map::new();
    for (name, ty, desc) in properties {
        props.insert(name.to_string(), json!({"type": ty, "description": desc}));
    }
    let mut root = Map::new();
    root.insert("type".to_string(), json!("object"));
    root.insert("properties".to_string(), Value::Object(props));
    if !required.is_empty() {
        root.insert("required".to_string(), json!(required));
    }
    if let Some(object) = extra.as_object() {
        for (key, value) in object {
            root.insert(key.clone(), value.clone());
        }
    }
    Arc::new(root)
}

const PROJECT_PROP: (&str, &str, &str) = (
    "project",
    "string",
    "Configured project id (light-show, ontrack, ...).",
);
const ARTIFACT_PROP: (&str, &str, &str) = (
    "artifact",
    "string",
    "Artifact path, project-relative or absolute inside the project root.",
);
const CHANNEL_PROP: (&str, &str, &str) = (
    "channel",
    "string",
    "Release channel: google-play or fdroid.",
);

fn tool(
    name: &'static str,
    description: &'static str,
    schema: Arc<Map<String, Value>>,
    read_only: bool,
    destructive: bool,
    idempotent: bool,
) -> Tool {
    Tool::new(name, description, schema).annotate(
        ToolAnnotations::new()
            .read_only(read_only)
            .destructive(destructive)
            .idempotent(idempotent)
            .open_world(false),
    )
}

fn tool_list() -> Vec<Tool> {
    vec![
        // ---- read class ----
        tool(
            "slipway_status",
            "Server status: configured projects and the receipt-log summary (counts, last actions, hashes a failed gate has marked ineligible). Status is a function of receipts, never of the tree alone.",
            schema(&[], &[], Value::Null),
            true,
            false,
            true,
        ),
        tool(
            "slipway_toolchain",
            "Read the Android toolchain census on this host: build-tools/apksigner/aapt2/adb/bundletool/keytool versions, ANDROID_HOME, attached devices.",
            schema(&[], &[], Value::Null),
            true,
            false,
            true,
        ),
        tool(
            "slipway_inspect",
            "Read one artifact: sha256, entry census, per-.so 16 KB alignment table, manifest facts read back from the artifact, and measured signature state.",
            schema(
                &[PROJECT_PROP, ARTIFACT_PROP],
                &["project", "artifact"],
                Value::Null,
            ),
            true,
            false,
            true,
        ),
        tool(
            "slipway_verify",
            "Verify one artifact's measured signature state against an expectation (signed/unsigned) and its signing eligibility (a hash marked ineligible by any failed gate is refused). Appends a verify receipt.",
            schema(
                &[
                    PROJECT_PROP,
                    ARTIFACT_PROP,
                    (
                        "expect",
                        "string",
                        "Expected signature state: signed, unsigned, or any (default).",
                    ),
                ],
                &["project", "artifact"],
                Value::Null,
            ),
            true,
            false,
            true,
        ),
        tool(
            "slipway_release_check",
            "Per-project release summary: latest certification records per channel, developer-verification registration posture, last receipted actions, ineligible hashes, and what is still owed (Owner signing step, portal submission).",
            schema(&[PROJECT_PROP], &["project"], Value::Null),
            true,
            false,
            true,
        ),
        tool(
            "slipway_certify",
            "Execute the publish-gate battery owned by the publish skills for one project + channel against a built artifact, and issue a certification record with per-gate measured evidence. The verdict is certified only when every gate passes; a refused gate is recorded, never skipped.",
            schema(
                &[PROJECT_PROP, CHANNEL_PROP, ARTIFACT_PROP],
                &["project", "channel", "artifact"],
                Value::Null,
            ),
            false,
            false,
            false,
        ),
        tool(
            "slipway_bundle_check",
            "Re-verify a release bundle: every file's sha256 against the bundle manifest, and the artifact hash still agreeing with the certification record it cites.",
            schema(
                &[("bundle", "string", "Bundle name or path.")],
                &["bundle"],
                Value::Null,
            ),
            true,
            false,
            true,
        ),
        // ---- execute class ----
        tool(
            "slipway_build",
            "Run the project's pinned build command (the S0 hand-verified invocation) and gate the outputs: every configured output must exist, be UNSIGNED (S2 builds are unsigned by construction), and is receipted with its hash.",
            schema(&[PROJECT_PROP], &["project"], Value::Null),
            false,
            false,
            false,
        ),
        tool(
            "slipway_install",
            "Install an artifact onto the local emulator (emulator serials only; physical devices are out of scope in this build). Receipted.",
            schema(
                &[
                    PROJECT_PROP,
                    ARTIFACT_PROP,
                    ("target", "string", "Emulator serial, e.g. emulator-5554."),
                ],
                &["project", "artifact", "target"],
                Value::Null,
            ),
            false,
            false,
            false,
        ),
        tool(
            "slipway_device_open",
            "Open a device session on the emulator for an artifact identified by a hash from slipway's own receipt log (never a free path).",
            schema(
                &[
                    PROJECT_PROP,
                    (
                        "artifact_hash",
                        "string",
                        "sha256 of a slipway-built artifact from the receipt log.",
                    ),
                    ("target", "string", "Emulator serial."),
                ],
                &["project", "artifact_hash", "target"],
                Value::Null,
            ),
            false,
            false,
            false,
        ),
        tool(
            "slipway_device_verify",
            "Verify the session's app on the device: installed, launch dispatched, process alive after launch, and no fatal crash for the package in the log window. Receipted; a failed check is a failed gate on the artifact hash.",
            schema(
                &[(
                    "session_id",
                    "string",
                    "Session id from slipway_device_open.",
                )],
                &["session_id"],
                Value::Null,
            ),
            false,
            false,
            false,
        ),
        tool(
            "slipway_device_close",
            "Close a device session. Receipted.",
            schema(
                &[(
                    "session_id",
                    "string",
                    "Session id from slipway_device_open.",
                )],
                &["session_id"],
                Value::Null,
            ),
            false,
            false,
            false,
        ),
        tool(
            "slipway_drive_app",
            "D-121 client half: connect as a modern-era MCP client (discover-first, no initialize) to the project's in-app MCP server and run its scripted QA scenario (light-show: list/load/place/evaluate + deterministic playtest; ONTrack: status + deterministic route optimization on the 1-degree fixture). Records the transcript.",
            schema(&[PROJECT_PROP], &["project"], Value::Null),
            false,
            false,
            false,
        ),
        // ---- consequential class ----
        tool(
            "slipway_sign",
            "Sign an artifact — gated by a fresh, single-use, Owner-signed manifest (schema slipway.sign-manifest/v1: project, artifact hash, channel, key reference, nonce, issue time). The keystore password reaches the signer only through a named environment variable; slipway never reads or stores it. A receipt-ineligible hash can never be signed. The output must then verify as signed before its hash is release-eligible.",
            schema(
                &[
                    PROJECT_PROP,
                    ARTIFACT_PROP,
                    (
                        "manifest",
                        "object",
                        "The Owner-signed sign manifest (slipway.sign-manifest/v1).",
                    ),
                ],
                &["project", "artifact", "manifest"],
                Value::Null,
            ),
            false,
            true,
            false,
        ),
        tool(
            "slipway_release",
            "Assemble the release-prep bundle for one project + channel: the artifact, its certification record, and the app's release documents, under a slipway.bundle/v1 manifest with per-file hashes. Produces the submission bundle as the deliverable — submission itself is the Owner's act and is never performed.",
            schema(
                &[
                    PROJECT_PROP,
                    CHANNEL_PROP,
                    ARTIFACT_PROP,
                    (
                        "certification",
                        "string",
                        "Certification record file name (in the state certifications dir) or path.",
                    ),
                ],
                &["project", "channel", "artifact", "certification"],
                Value::Null,
            ),
            false,
            false,
            false,
        ),
    ]
}

fn ok_json(value: Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
    )])
}

fn err_json(failure: &Failure) -> CallToolResult {
    let mut result = CallToolResult::error(vec![ContentBlock::text(
        serde_json::to_string_pretty(&json!({"code": failure.code, "message": failure.message}))
            .unwrap_or_default(),
    )]);
    result.structured_content = Some(json!({
        "error": {"code": failure.code, "message": failure.message}
    }));
    result
}

#[derive(Clone)]
pub struct SlipwayServer {
    config: Arc<Config>,
    tool_list: Arc<Vec<Tool>>,
}

impl SlipwayServer {
    pub fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
            tool_list: Arc::new(tool_list()),
        }
    }

    /// Run the server over a duplex stream pair guarded by the
    /// era guard (see `crate::guard`).
    pub async fn run<R, W>(self, reader: R, writer: W) -> Result<(), Box<dyn std::error::Error>>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let transport = tokio::io::join(reader, writer);
        let service = match self.serve(transport).await {
            Ok(service) => service,
            // A client that closes before selecting a lifecycle
            // (a probe, a spawn check, a client that only ever
            // wanted the guard's refusal) is a clean end of
            // session, not a server failure.
            Err(rmcp::service::ServerInitializeError::ConnectionClosed(_)) => {
                return Ok(());
            }
            Err(e) => return Err(Box::new(e) as Box<dyn std::error::Error>),
        };
        service.waiting().await?;
        Ok(())
    }
}

impl ServerHandler for SlipwayServer {
    fn get_info(&self) -> InitializeResult {
        let mut server_info = Implementation::default();
        server_info.name = "slipway".to_string();
        server_info.version = env!("CARGO_PKG_VERSION").to_string();
        let mut info = InitializeResult::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = server_info;
        info
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2026_07_28])
    }

    async fn list_tools(
        &self,
        _params: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: (*self.tool_list).clone(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let args = request
            .arguments
            .as_ref()
            .map(|m| Value::Object(m.clone()))
            .unwrap_or_else(|| json!({}));
        if !args.is_object() {
            return Err(McpError::invalid_params(
                "arguments must be an object",
                None,
            ));
        }
        if request.name.as_ref() == "slipway_drive_app" {
            let config = self.config.clone();
            let project_id = match args.get("project").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => {
                    return Err(McpError::invalid_params(
                        "missing string argument: project",
                        None,
                    ));
                }
            };
            return Ok(match config.project(&project_id) {
                Ok(project) => match client::drive_app(project).await {
                    Ok(transcript) => {
                        let mut record = json!({
                            "action": "device_session",
                            "project": project.id,
                            "actor": "slipway",
                            "started": crate::exec::now_rfc3339(),
                            "command": project.app_server_command,
                            "gates": [{
                                "gate": "app_qa_scenario",
                                "result": if transcript["all_passed"].as_bool() == Some(true) { "pass" } else { "fail" },
                                "detail": format!("{} of {} steps passed",
                                    transcript["steps_passed"], transcript["steps_total"]),
                            }],
                            "result": if transcript["all_passed"].as_bool() == Some(true) { "ok" } else { "failure" },
                            "unsigned": true,
                        });
                        if let Err(e) = receipt::append(&config.receipts_path(), std::mem::take(&mut record)) {
                            return Ok(err_json(&e).into());
                        }
                        ok_json(transcript)
                    }
                    Err(failure) => err_json(&failure),
                },
                Err(failure) => {
                    if failure.is_params {
                        return Err(McpError::invalid_params(failure.message.clone(), None));
                    }
                    err_json(&failure)
                }
            }
            .into());
        }
        let config = self.config.clone();
        let name = request.name.to_string();
        let result =
            tokio::task::spawn_blocking(move || tools::dispatch(&config, &name, &args)).await;
        match result {
            Ok(Ok(value)) => Ok(ok_json(value).into()),
            Ok(Err(failure)) if failure.is_params => {
                Err(McpError::invalid_params(failure.message, None))
            }
            Ok(Err(failure)) => Ok(err_json(&failure).into()),
            Err(join) => Ok(err_json(&Failure::domain(
                "internal",
                format!("tool task failed: {join}"),
            ))
            .into()),
        }
    }
}
