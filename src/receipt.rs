//! The `slipway.receipt` v1 append-only JSONL log (schema
//! specified at S0, `~/workspace/slipway-20261021/S0-PINDOWN.md`
//! §6). Receipts are the only source `slipway_status` reads —
//! status never guesses. A failed gate marks its output hash
//! ineligible for signing, permanently: re-asking cannot
//! re-eligible it, so the ineligible set is derived from the log.

use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};

use crate::model::Failure;

/// Append one receipt line. `fields` carries the action-specific
/// record; the envelope (schema, timestamps) is added here.
pub fn append(path: &Path, fields: Value) -> Result<(), Failure> {
    let mut record = fields;
    record["schema"] = json!("slipway.receipt/v1");
    if record.get("finished").is_none() {
        record["finished"] = json!(crate::exec::now_rfc3339());
    }
    let line = serde_json::to_string(&record)
        .map_err(|e| Failure::domain("receipt_failed", format!("receipt encode failed: {e}")))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| {
            Failure::domain(
                "receipt_failed",
                format!("cannot open receipt log {}: {e}", path.display()),
            )
        })?;
    writeln!(file, "{line}")
        .map_err(|e| Failure::domain("receipt_failed", format!("cannot append receipt: {e}")))?;
    Ok(())
}

/// Read all receipts (missing log = empty history).
pub fn read_all(path: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

/// The most recent receipt per (action, project) plus the set of
/// output hashes any failed gate has marked ineligible.
pub fn summarize(path: &Path) -> Value {
    let receipts = read_all(path);
    let mut last: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut ineligible: Vec<String> = Vec::new();
    let mut total = 0usize;
    for receipt in &receipts {
        total += 1;
        let action = receipt["action"].as_str().unwrap_or("?");
        let project = receipt["project"].as_str().unwrap_or("?");
        last.insert(format!("{project}:{action}"), receipt.clone());
        if receipt["result"].as_str() == Some("failure")
            && let Some(hash) = receipt["output_hash"].as_str()
        {
            ineligible.push(hash.to_string());
        }
        if let Some(gates) = receipt["gates"].as_array() {
            for gate in gates {
                if gate["result"].as_str() == Some("fail")
                    && let Some(hash) = receipt["output_hash"].as_str()
                {
                    ineligible.push(hash.to_string());
                }
            }
        }
    }
    ineligible.sort();
    ineligible.dedup();
    json!({
        "receipt_count": total,
        "last_by_project_action": Value::Object(last),
        "ineligible_hashes": ineligible,
    })
}

/// Is this artifact hash barred from signing by a failed gate?
pub fn is_ineligible(path: &Path, hash: &str) -> bool {
    summarize(path)["ineligible_hashes"]
        .as_array()
        .is_some_and(|list| list.iter().any(|h| h.as_str() == Some(hash)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_gate_marks_hash_ineligible() {
        let tmp = std::env::temp_dir().join(format!("slipway-rcpt-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("mkdir");
        let log = tmp.join("receipts.jsonl");
        append(
            &log,
            json!({"action": "verify", "project": "x", "result": "failure",
                   "output_hash": "abc123", "gates": []}),
        )
        .expect("append");
        append(
            &log,
            json!({"action": "build", "project": "x", "result": "ok",
                   "output_hash": "def456", "gates": [{"gate": "g", "result": "pass"}]}),
        )
        .expect("append");
        assert!(is_ineligible(&log, "abc123"));
        assert!(!is_ineligible(&log, "def456"));
        let summary = summarize(&log);
        assert_eq!(summary["receipt_count"], 2);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
