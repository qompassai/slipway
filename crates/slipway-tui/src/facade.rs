//! The narrow surface the TUI drives.
//!
//! [`SlipwayFacade`] exposes exactly what the screens need:
//! `status`, `tools`, and one generic `call`. Two real
//! implementations exist: [`LibFacade`] (in-process, over the
//! `slipway` library's `tools::dispatch` — preferred where the
//! library is reachable) and [`StdioFacade`] (an MCP stdio
//! client spawning the member's own server binary, speaking the
//! modern discover-first wire exactly as the member's wire tests
//! do). Tests use [`FakeFacade`], which is scripted and records
//! every call.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use slipway::config::Config;
use slipway::tools;

/// Largest single stdio response frame the fallback client will
/// accept, in bytes. A longer frame is a dead server, not data.
pub const RESPONSE_BYTES_MAX: usize = 4 * 1024 * 1024;

/// A facade failure. `ServerDied` is kept distinct because the
/// pipeline state machine must render it as *unknown*, never as
/// an ordinary step failure or (worst) a success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FacadeError {
    /// The server answered with an error, or the call was
    /// refused locally with a reason.
    Message(String),
    /// The transport is gone: EOF, spawn failure, oversized
    /// frame, or a write to a closed pipe.
    ServerDied(String),
}

impl std::fmt::Display for FacadeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FacadeError::Message(m) => write!(f, "{m}"),
            FacadeError::ServerDied(m) => write!(f, "server died: {m}"),
        }
    }
}

impl std::error::Error for FacadeError {}

/// The operations the TUI needs from slipway.
pub trait SlipwayFacade: Send {
    /// The `slipway_status` report.
    fn status(&mut self) -> Result<Value, FacadeError>;
    /// The member's tool list (one JSON object per tool).
    fn tools(&mut self) -> Result<Vec<Value>, FacadeError>;
    /// Call one tool by name with JSON arguments.
    fn call(&mut self, tool: &str, args: Value) -> Result<Value, FacadeError>;
    /// Whether the underlying transport is believed alive.
    fn alive(&self) -> bool;
}

/// The tool names the member's server advertises, in surface
/// order (mirrors `slipway::server`'s tool list and the member's
/// wire test, which pins exactly these 15).
pub const TOOL_NAMES: [&str; 15] = [
    "slipway_status",
    "slipway_toolchain",
    "slipway_inspect",
    "slipway_verify",
    "slipway_release_check",
    "slipway_certify",
    "slipway_bundle_check",
    "slipway_build",
    "slipway_install",
    "slipway_device_open",
    "slipway_device_verify",
    "slipway_device_close",
    "slipway_drive_app",
    "slipway_sign",
    "slipway_release",
];

fn tool_objects() -> Vec<Value> {
    TOOL_NAMES.iter().map(|n| json!({"name": n})).collect()
}

// ----------------------------------------------------------------
// FakeFacade
// ----------------------------------------------------------------

/// One recorded facade call: `(tool, args)`.
pub type RecordedCalls = Arc<Mutex<Vec<(String, Value)>>>;

/// An in-memory [`SlipwayFacade`] for tests: scripted results,
/// recorded calls, and a scriptable mid-call server death.
#[derive(Debug, Default)]
pub struct FakeFacade {
    /// Result returned by `status`.
    pub status_result: Value,
    /// Tools returned by `tools`.
    pub tools_result: Vec<Value>,
    /// Per-tool queued results, consumed in order; when a tool's
    /// queue is empty the default result answers.
    pub call_results: HashMap<String, VecDeque<Result<Value, FacadeError>>>,
    /// Tool that kills the server when called (that call and
    /// every later call report [`FacadeError::ServerDied`]).
    pub die_on_call: Option<String>,
    /// Every call made, in order — shared so a test can inspect
    /// it after the facade has moved into the app.
    pub calls: RecordedCalls,
    alive: bool,
}

impl FakeFacade {
    /// A fake with a plausible status, the real 15-tool surface,
    /// and an all-green scripted certification.
    pub fn new() -> FakeFacade {
        FakeFacade {
            status_result: json!({
                "server": "slipway",
                "version": "0.1.0",
                "projects": [{"id": "demo", "name": "Demo",
                              "artifacts": ["app.aab"]}],
                "receipts": {"total": 0},
            }),
            tools_result: tool_objects(),
            call_results: HashMap::new(),
            die_on_call: None,
            calls: Arc::new(Mutex::new(Vec::new())),
            alive: true,
        }
    }

    /// Queue one scripted result for the next call to `tool`.
    pub fn queue_call_result(&mut self, tool: &str, result: Result<Value, FacadeError>) {
        self.call_results
            .entry(tool.to_string())
            .or_default()
            .push_back(result);
    }

    /// A snapshot of every recorded call, in order.
    pub fn calls_snapshot(&self) -> Vec<(String, Value)> {
        match self.calls.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl SlipwayFacade for FakeFacade {
    fn status(&mut self) -> Result<Value, FacadeError> {
        if !self.alive {
            return Err(FacadeError::ServerDied("fake server is dead".into()));
        }
        Ok(self.status_result.clone())
    }

    fn tools(&mut self) -> Result<Vec<Value>, FacadeError> {
        if !self.alive {
            return Err(FacadeError::ServerDied("fake server is dead".into()));
        }
        Ok(self.tools_result.clone())
    }

    fn call(&mut self, tool: &str, args: Value) -> Result<Value, FacadeError> {
        if let Ok(mut guard) = self.calls.lock() {
            guard.push((tool.to_string(), args));
        }
        if !self.alive || self.die_on_call.as_deref() == Some(tool) {
            self.alive = false;
            return Err(FacadeError::ServerDied(format!(
                "connection lost during {tool}"
            )));
        }
        if let Some(queue) = self.call_results.get_mut(tool)
            && let Some(result) = queue.pop_front()
        {
            return result;
        }
        Ok(json!({"ok": true, "tool": tool}))
    }

    fn alive(&self) -> bool {
        self.alive
    }
}

// ----------------------------------------------------------------
// LibFacade
// ----------------------------------------------------------------

/// The real facade over the member's library: every call goes
/// through `slipway::tools::dispatch` with a loaded [`Config`].
/// No keystore is touched by this facade; signing through the
/// library stays behind slipway's own manifest gates.
pub struct LibFacade {
    config: Config,
}

impl LibFacade {
    /// Load the member config at `path` (the same file the
    /// server binary would take via `--config`).
    pub fn load(path: &std::path::Path) -> Result<LibFacade, FacadeError> {
        let config = Config::load(path)
            .map_err(|f| FacadeError::Message(format!("[{}] {}", f.code, f.message)))?;
        Ok(LibFacade { config })
    }
}

impl SlipwayFacade for LibFacade {
    fn status(&mut self) -> Result<Value, FacadeError> {
        self.call("slipway_status", json!({}))
    }

    fn tools(&mut self) -> Result<Vec<Value>, FacadeError> {
        // The library exposes no tool-list accessor; the surface
        // is pinned by the member's own wire test, mirrored here.
        Ok(tool_objects())
    }

    fn call(&mut self, tool: &str, args: Value) -> Result<Value, FacadeError> {
        tools::dispatch(&self.config, tool, &args)
            .map_err(|f| FacadeError::Message(format!("[{}] {}", f.code, f.message)))
    }

    fn alive(&self) -> bool {
        true
    }
}

// ----------------------------------------------------------------
// StdioFacade (MCP stdio client fallback)
// ----------------------------------------------------------------

fn meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo":
            {"name": "slipway-tui", "version": env!("CARGO_PKG_VERSION")},
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

fn payload_of(result: &Value) -> Result<Value, FacadeError> {
    if result["isError"].as_bool() == Some(true) {
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or("tool error")
            .to_string();
        return Err(FacadeError::Message(text));
    }
    if let Some(structured) = result.get("structuredContent")
        && !structured.is_null()
    {
        return Ok(structured.clone());
    }
    if let Some(text) = result["content"][0]["text"].as_str() {
        return Ok(serde_json::from_str(text).unwrap_or_else(|_| json!({"text": text})));
    }
    Ok(Value::Null)
}

/// The fallback facade: spawns the member's server binary and
/// speaks the modern discover-first wire (no `initialize` —
/// the era guard refuses it by design).
pub struct StdioFacade {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    next_id: i64,
    alive: bool,
}

impl StdioFacade {
    /// Spawn `argv` (the server command, e.g.
    /// `["slipway", "--config", "…"]`) and run `server/discover`.
    pub fn spawn(argv: &[String]) -> Result<StdioFacade, FacadeError> {
        if argv.is_empty() {
            return Err(FacadeError::Message("empty server command".into()));
        }
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| FacadeError::ServerDied(format!("cannot spawn {}: {e}", argv[0])))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| FacadeError::ServerDied("server has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| FacadeError::ServerDied("server has no stdout".into()))?;
        let mut facade = StdioFacade {
            child,
            stdin,
            reader: BufReader::new(stdout),
            next_id: 0,
            alive: true,
        };
        facade.rpc("server/discover", json!({"_meta": meta()}))?;
        Ok(facade)
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value, FacadeError> {
        if !self.alive {
            return Err(FacadeError::ServerDied("transport already dead".into()));
        }
        self.next_id += 1;
        let id = self.next_id;
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method,
                           "params": params});
        let mut line = serde_json::to_string(&frame)
            .map_err(|e| FacadeError::Message(format!("cannot encode frame: {e}")))?;
        line.push('\n');
        if let Err(e) = self
            .stdin
            .write_all(line.as_bytes())
            .and_then(|()| self.stdin.flush())
        {
            self.alive = false;
            return Err(FacadeError::ServerDied(format!("write failed: {e}")));
        }
        loop {
            let mut buf = String::new();
            match self.reader.read_line(&mut buf) {
                Ok(0) => {
                    self.alive = false;
                    return Err(FacadeError::ServerDied("server closed the wire".into()));
                }
                Ok(_) => {
                    if buf.len() > RESPONSE_BYTES_MAX {
                        self.alive = false;
                        return Err(FacadeError::ServerDied("oversized frame".into()));
                    }
                    let Ok(msg) = serde_json::from_str::<Value>(&buf) else {
                        continue; // a non-JSON line is not our reply
                    };
                    if msg["id"] == json!(id) {
                        if let Some(error) = msg.get("error") {
                            return Err(FacadeError::Message(format!(
                                "rpc error {}: {}",
                                error["code"], error["message"]
                            )));
                        }
                        return Ok(msg["result"].clone());
                    }
                }
                Err(e) => {
                    self.alive = false;
                    return Err(FacadeError::ServerDied(format!("read failed: {e}")));
                }
            }
        }
    }
}

impl SlipwayFacade for StdioFacade {
    fn status(&mut self) -> Result<Value, FacadeError> {
        self.call("slipway_status", json!({}))
    }

    fn tools(&mut self) -> Result<Vec<Value>, FacadeError> {
        let result = self.rpc("tools/list", json!({"_meta": meta()}))?;
        let tools = result["tools"]
            .as_array()
            .ok_or_else(|| FacadeError::Message("tools/list returned no tools".into()))?;
        Ok(tools.clone())
    }

    fn call(&mut self, tool: &str, args: Value) -> Result<Value, FacadeError> {
        let result = self.rpc(
            "tools/call",
            json!({"_meta": meta(), "name": tool, "arguments": args}),
        )?;
        payload_of(&result)
    }

    fn alive(&self) -> bool {
        // The flag is authoritative: every transport failure
        // path in `rpc` clears it, and a dead child surfaces as
        // an EOF on the next read, which clears it then.
        self.alive
    }
}

impl Drop for StdioFacade {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Build a config map for tests that need a [`LibFacade`]-shaped
/// argument set without a filesystem config. (Kept tiny: the
/// facade argument shapes the workflows send.)
pub fn step_args(tool: &str, project: &str, channel: &str, artifact: &str) -> Value {
    let mut map = BTreeMap::new();
    map.insert("project".to_string(), json!(project));
    match tool {
        "slipway_build" => {}
        "slipway_sign" | "slipway_verify" | "slipway_install" => {
            map.insert("artifact".to_string(), json!(artifact));
        }
        "slipway_release" => {
            map.insert("channel".to_string(), json!(channel));
        }
        _ => {}
    }
    Value::Object(map.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_records_calls_and_answers_status() {
        let mut fake = FakeFacade::new();
        assert_eq!(
            fake.status().map(|v| v["version"].clone()),
            Ok(json!("0.1.0"))
        );
        let tools = fake.tools().expect("tools");
        assert_eq!(tools.len(), 15);
        let out = fake.call("slipway_build", json!({"project": "demo"}));
        assert!(out.is_ok());
        assert_eq!(fake.calls_snapshot().len(), 1);
    }

    #[test]
    fn fake_death_is_sticky_and_typed() {
        let mut fake = FakeFacade::new();
        fake.die_on_call = Some("slipway_build".to_string());
        let first = fake.call("slipway_build", json!({}));
        assert!(matches!(first, Err(FacadeError::ServerDied(_))));
        let second = fake.call("slipway_verify", json!({}));
        assert!(matches!(second, Err(FacadeError::ServerDied(_))));
        assert!(!fake.alive());
    }

    #[test]
    fn step_args_carry_only_what_each_tool_needs() {
        let sign = step_args("slipway_sign", "p", "internal", "a.aab");
        assert_eq!(sign["project"], json!("p"));
        assert_eq!(sign["artifact"], json!("a.aab"));
        assert!(sign.get("channel").is_none());
        let release = step_args("slipway_release", "p", "internal", "a.aab");
        assert_eq!(release["channel"], json!("internal"));
    }
}
