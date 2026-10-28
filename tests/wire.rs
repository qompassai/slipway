//! Wire-level integration tests against the real binary:
//! modern discover, the 15-tool surface, the era guard
//! (legacy initialize refused -32022, unknown method -32601,
//! malformed frame -32700), and the argument taxonomy
//! (missing argument = -32602). Responses are correlated by
//! id — rmcp dispatches concurrently, so order is not assumed.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Session {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Session {
    fn spawn() -> Self {
        let config = format!("{}/config.sandbox.toml", env!("CARGO_MANIFEST_DIR"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_slipway"))
            .args(["--config", &config])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn slipway");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        Session {
            child,
            stdin,
            reader: BufReader::new(stdout),
            next_id: 0,
        }
    }

    fn send(&mut self, frame: &Value) {
        let mut line = serde_json::to_string(frame).unwrap();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn send_raw(&mut self, raw: &str) {
        self.stdin.write_all(raw.as_bytes()).unwrap();
        self.stdin.flush().unwrap();
    }

    /// Read frames until one carries `id` (or a null id for
    /// guard-answered parse errors); return it.
    fn await_id(&mut self, id: Option<i64>) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(Instant::now() < deadline, "timed out awaiting id {id:?}");
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).expect("read frame");
            assert!(n > 0, "server closed the wire awaiting id {id:?}");
            let frame: Value = serde_json::from_str(&line)
                .unwrap_or_else(|_| panic!("non-JSON frame from server: {line}"));
            match id {
                Some(want) if frame["id"] == json!(want) => return frame,
                None if frame["id"].is_null() => return frame,
                _ => continue,
            }
        }
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        }));
        self.await_id(Some(id))
    }
}

fn meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "slipway-wire-test", "version": "0.1.0"},
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

#[test]
fn wire_modern_surface_and_guard() {
    let mut s = Session::spawn();

    // Discover: modern era only, correct serverInfo from birth.
    let discover = s.rpc("server/discover", json!({"_meta": meta()}));
    let result = &discover["result"];
    assert_eq!(
        result["supportedVersions"],
        json!(["2026-07-28"]),
        "discover must advertise exactly the modern era: {discover}"
    );
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        json!("slipway"),
        "serverInfo rides the discover _meta: {discover}"
    );

    // The 15-tool surface, read class first, consequential last.
    let list = s.rpc("tools/list", json!({"_meta": meta()}));
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
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
        ]
    );
    // Annotations from birth on every tool.
    for tool in list["result"]["tools"].as_array().unwrap() {
        assert!(
            tool["annotations"]["readOnlyHint"].is_boolean(),
            "tool {} lacks a readOnlyHint annotation",
            tool["name"]
        );
    }

    // A real read call.
    let status = s.rpc(
        "tools/call",
        json!({"_meta": meta(), "name": "slipway_status", "arguments": {}}),
    );
    assert!(
        status["result"]["isError"].as_bool() != Some(true),
        "slipway_status errored: {status}"
    );

    // Argument-shape failure is a protocol error (-32602).
    let bad_args = s.rpc(
        "tools/call",
        json!({"_meta": meta(), "name": "slipway_inspect", "arguments": {}}),
    );
    assert_eq!(bad_args["error"]["code"], json!(-32602), "{bad_args}");

    // Legacy initialize is refused by the era guard (-32022).
    let legacy = s.rpc(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "legacy-probe", "version": "0"},
        }),
    );
    assert_eq!(legacy["error"]["code"], json!(-32022), "{legacy}");

    // Unknown method -32601.
    let unknown = s.rpc("bogus/method", json!({"_meta": meta()}));
    assert_eq!(unknown["error"]["code"], json!(-32601), "{unknown}");

    // Malformed frame -32700 (answered with a null id).
    s.send_raw("this is not json\n");
    let parse = s.await_id(None);
    assert_eq!(parse["error"]["code"], json!(-32700), "{parse}");

    drop(s.stdin);
    let _ = s.child.wait();
}

#[test]
fn wire_refuses_unknown_project_and_outside_artifact() {
    let mut s = Session::spawn();
    let _ = s.rpc("server/discover", json!({"_meta": meta()}));
    // Unknown project id is an argument-value failure (-32602).
    let unknown = s.rpc(
        "tools/call",
        json!({"_meta": meta(), "name": "slipway_inspect",
               "arguments": {"project": "nope", "artifact": "x.apk"}}),
    );
    assert_eq!(
        unknown["error"]["code"],
        json!(-32602),
        "unknown project must be refused as an invalid parameter: {unknown}"
    );
    // An artifact path escaping the project root is refused.
    let escape = s.rpc(
        "tools/call",
        json!({"_meta": meta(), "name": "slipway_inspect",
               "arguments": {"project": "light-show", "artifact": "/etc/passwd"}}),
    );
    assert_eq!(
        escape["result"]["isError"],
        json!(true),
        "outside-root artifact must be refused: {escape}"
    );
    drop(s.stdin);
    let _ = s.child.wait();
}
