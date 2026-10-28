//! Tool dispatch: every slipway tool behind one entry point.
//! Read-class tools land first in the surface order, execute
//! next, consequential last (the program's class order).
//! Every action appends its `slipway.receipt` v1 line before
//! returning — receipts are memory; a failure is a receipt too.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use crate::android;
use crate::artifact;
use crate::certify;
use crate::config::{Config, Project};
use crate::exec;
use crate::model::{Failure, ToolResult};
use crate::receipt;
use crate::release;
use crate::signing;

fn arg_str(args: &Value, key: &str) -> Result<String, Failure> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| Failure::params(format!("missing string argument: {key}")))
}

fn arg_str_opt(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(|s| s.to_string())
}

fn project_of<'c>(config: &'c Config, args: &Value) -> Result<&'c Project, Failure> {
    let id = arg_str(args, "project")?;
    config.project(&id)
}

/// Resolve an artifact argument (project-relative or absolute)
/// and firewall-check it.
fn artifact_of(config: &Config, project: &Project, args: &Value) -> Result<PathBuf, Failure> {
    let raw = arg_str(args, "artifact")?;
    let path = if PathBuf::from(&raw).is_absolute() {
        PathBuf::from(&raw)
    } else {
        project.root.join(&raw)
    };
    config.check_artifact_path(project, &path)
}

fn git_tree(root: &std::path::Path) -> Value {
    let head = exec::run(
        &[
            "git".to_string(),
            "-C".to_string(),
            root.to_string_lossy().into_owned(),
            "rev-parse".to_string(),
            "HEAD".to_string(),
        ],
        None,
        &BTreeMap::new(),
        Duration::from_secs(30),
    );
    let status = exec::run(
        &[
            "git".to_string(),
            "-C".to_string(),
            root.to_string_lossy().into_owned(),
            "status".to_string(),
            "--porcelain".to_string(),
        ],
        None,
        &BTreeMap::new(),
        Duration::from_secs(30),
    );
    json!({
        "head": head.stdout.trim(),
        "status_hash": exec::sha256_bytes(status.stdout.as_bytes()),
    })
}

fn base_receipt(action: &str, project: &Project) -> Value {
    json!({
        "action": action,
        "project": project.id,
        "actor": "slipway",
        "started": exec::now_rfc3339(),
        "command": [],
        "input_tree": git_tree(&project.root),
        "input_hashes": {},
        "output_hash": null,
        "output_size": null,
        "gates": [],
        "result": "ok",
        "unsigned": true,
    })
}

pub fn dispatch(config: &Config, name: &str, args: &Value) -> ToolResult {
    match name {
        "slipway_status" => status(config),
        "slipway_toolchain" => Ok(android::toolchain_census(config)),
        "slipway_inspect" => inspect(config, args),
        "slipway_verify" => verify(config, args),
        "slipway_release_check" => release_check(config, args),
        "slipway_certify" => certify_tool(config, args),
        "slipway_build" => build(config, args),
        "slipway_install" => install(config, args),
        "slipway_device_open" => device_open(config, args),
        "slipway_device_verify" => device_verify(config, args),
        "slipway_device_close" => device_close(config, args),
        "slipway_sign" => sign_tool(config, args),
        "slipway_release" => release_tool(config, args),
        "slipway_bundle_check" => bundle_check(config, args),
        other => Err(Failure::params(format!("unknown tool: {other}"))),
    }
}

fn status(config: &Config) -> ToolResult {
    let projects: Vec<Value> = config
        .projects
        .iter()
        .map(|p| {
            json!({
                "id": p.id, "name": p.name, "package": p.package,
                "root": p.root, "artifacts": p.artifacts,
            })
        })
        .collect();
    Ok(json!({
        "server": "slipway",
        "version": env!("CARGO_PKG_VERSION"),
        "state_dir": config.state_root,
        "projects": projects,
        "receipts": receipt::summarize(&config.receipts_path()),
    }))
}

fn inspect(config: &Config, args: &Value) -> ToolResult {
    let project = project_of(config, args)?;
    let path = artifact_of(config, project, args)?;
    let census = artifact::census(&path)?;
    let manifest = android::manifest_facts(config, &path);
    let signature = android::signature_facts(config, &path);
    Ok(json!({
        "artifact": path,
        "sha256": census.sha256,
        "size": census.size,
        "entry_count": census.entries.len(),
        "has_signature_block": census.has_signature_block,
        "alignment": artifact::alignment_table(&census),
        "manifest": match &manifest {
            Ok(facts) => facts.to_json(),
            Err(e) => json!({"error": e}),
        },
        "signature": {
            "state": signature.state,
            "signer_sha256": signature.signer_sha256,
            "detail": signature.detail,
        },
    }))
}

fn verify(config: &Config, args: &Value) -> ToolResult {
    let project = project_of(config, args)?;
    let path = artifact_of(config, project, args)?;
    let expect = arg_str_opt(args, "expect").unwrap_or_else(|| "any".to_string());
    let census = artifact::census(&path)?;
    let signature = android::signature_facts(config, &path);
    let mut gates = Vec::new();
    let state_ok = expect == "any" || signature.state == expect;
    gates.push(json!({
        "gate": "signature_state",
        "result": if state_ok { "pass" } else { "fail" },
        "detail": format!("measured {}, expected {}", signature.state, expect),
    }));
    let ineligible = receipt::is_ineligible(&config.receipts_path(), &census.sha256);
    gates.push(json!({
        "gate": "signing_eligibility",
        "result": if ineligible { "fail" } else { "pass" },
        "detail": if ineligible { "a failed gate has marked this hash ineligible" } else { "no failed gate names this hash" },
    }));
    let ok = state_ok && !ineligible;
    let mut record = base_receipt("verify", project);
    record["output_hash"] = json!(census.sha256);
    record["output_size"] = json!(census.size);
    record["gates"] = json!(gates);
    record["result"] = json!(if ok { "ok" } else { "failure" });
    record["unsigned"] = json!(signature.state != "signed");
    receipt::append(&config.receipts_path(), record)?;
    if ok {
        Ok(json!({
            "artifact": path, "sha256": census.sha256,
            "signature": {"state": signature.state, "signer_sha256": signature.signer_sha256, "detail": signature.detail},
            "gates": gates, "verdict": "verified",
        }))
    } else {
        Err(Failure::domain(
            "verification_failed",
            format!(
                "artifact {} failed verification (signature {}, ineligible {ineligible})",
                path.display(),
                signature.state
            ),
        ))
    }
}

fn release_check(config: &Config, args: &Value) -> ToolResult {
    let project = project_of(config, args)?;
    // Latest certification per channel, from the records on disk.
    let mut certifications = Vec::new();
    if let Ok(read) = std::fs::read_dir(config.certifications_dir()) {
        for entry in read.flatten() {
            if let Ok(text) = std::fs::read_to_string(entry.path())
                && let Ok(value) = serde_json::from_str::<Value>(&text)
                && value["project"].as_str() == Some(project.id.as_str())
            {
                certifications.push(json!({
                    "channel": value["channel"],
                    "verdict": value["verdict"],
                    "issued": value["issued"],
                    "record": entry.path(),
                }));
            }
        }
    }
    let summary = receipt::summarize(&config.receipts_path());
    Ok(json!({
        "project": project.id,
        "package": project.package,
        "certifications": certifications,
        "developer_verification_registered": project.developer_verification_registered,
        "last_actions": summary["last_by_project_action"],
        "ineligible_hashes": summary["ineligible_hashes"],
        "owed": [
            "Owner signing step (no slipway keystore is configured; signing is manifest-gated, Owner-stamped)",
            "Portal submission (always the Owner's act)",
        ],
    }))
}

fn certify_tool(config: &Config, args: &Value) -> ToolResult {
    let project = project_of(config, args)?;
    let channel = arg_str(args, "channel")?;
    let path = artifact_of(config, project, args)?;
    let certification = certify::certify(config, project, &channel, &path)?;
    let record = certification.to_json();
    std::fs::create_dir_all(config.certifications_dir())
        .map_err(|e| Failure::domain("io", format!("cannot create certifications dir: {e}")))?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
    let record_path = config
        .certifications_dir()
        .join(format!("{}-{}-{}.json", project.id, channel, stamp));
    std::fs::write(
        &record_path,
        serde_json::to_string_pretty(&record).unwrap_or_default(),
    )
    .map_err(|e| Failure::domain("io", format!("cannot write certification record: {e}")))?;
    let mut receipt_record = base_receipt("release_check", project);
    receipt_record["output_hash"] = json!(record["artifact"]["sha256"]);
    receipt_record["gates"] = record["gates"].clone();
    receipt_record["result"] = json!(if certification.verdict() == "certified" {
        "ok"
    } else {
        "failure"
    });
    receipt_record["certification"] = json!({
        "channel": channel, "verdict": certification.verdict(), "record": record_path,
    });
    receipt::append(&config.receipts_path(), receipt_record)?;
    let mut out = record;
    out["record"] = json!(record_path);
    Ok(out)
}

fn build(config: &Config, args: &Value) -> ToolResult {
    let project = project_of(config, args)?;
    if project.build_command.is_empty() {
        return Err(Failure::domain(
            "build_unconfigured",
            format!("project {} has no build command configured", project.id),
        ));
    }
    let started = exec::now_rfc3339();
    let outcome = exec::run(
        &project.build_command,
        Some(&project.root),
        &project.build_env,
        Duration::from_secs(exec::BUILD_TIMEOUT_SECS),
    );
    let mut record = base_receipt("build", project);
    record["started"] = json!(started);
    record["command"] = json!(outcome.argv);
    let mut gates = vec![json!({
        "gate": "build_exit",
        "result": if outcome.ok() { "pass" } else { "fail" },
        "detail": format!("exit {:?} in {} ms{}", outcome.exit_code, outcome.elapsed_ms,
            if outcome.timed_out { " (TIMED OUT)" } else { "" }),
    })];
    let mut outputs = Vec::new();
    let mut first_hash: Option<String> = None;
    let mut all_unsigned = true;
    if outcome.ok() {
        for rel in &project.artifacts {
            let path = project.root.join(rel);
            if !path.is_file() {
                gates.push(json!({
                    "gate": "output_present", "result": "fail",
                    "detail": format!("expected build output {rel} is missing"),
                }));
                continue;
            }
            let hash = exec::sha256_file(&path)
                .map_err(|e| Failure::domain("io", format!("cannot hash build output: {e}")))?;
            let signature = android::signature_facts(config, &path);
            let debug_signed = signature.detail.contains("Android Debug")
                || signature
                    .signer_sha256
                    .as_deref()
                    .map(|f| {
                        f.starts_with(
                            "a40da80a59d170caa950cf15c18c454d47a39b26989d8b640ecd745ba71bf5dc",
                        )
                    })
                    .unwrap_or(false);
            let class = match signature.state.as_str() {
                "unsigned" => "unsigned",
                "signed" if debug_signed => "debug-only",
                "signed" => "signed",
                _ => "unverified",
            };
            if class != "unsigned" {
                all_unsigned = false;
            }
            gates.push(json!({
                "gate": "unsigned_output", "result": if class == "unsigned" { "pass" } else { "fail" },
                "detail": format!("{rel} is {class} — S2 builds are unsigned by construction; a debug-signed output is classified debug-only and ineligible downstream, a release-signed output fails the build gate"),
            }));
            if first_hash.is_none() {
                first_hash = Some(hash.clone());
            }
            outputs.push(json!({
                "path": path, "sha256": hash, "signature_state": signature.state,
                "classification": class,
            }));
        }
    }
    let ok = outcome.ok() && gates.iter().all(|g| g["result"].as_str() == Some("pass"));
    record["gates"] = json!(gates);
    record["result"] = json!(if ok { "ok" } else { "failure" });
    record["output_hash"] = json!(first_hash);
    record["unsigned"] = json!(all_unsigned);
    record["outputs"] = json!(outputs);
    receipt::append(&config.receipts_path(), record)?;
    if ok {
        Ok(json!({
            "project": project.id, "outputs": outputs,
            "elapsed_ms": outcome.elapsed_ms, "verdict": "built-unsigned",
        }))
    } else if outcome.ok() {
        // The command succeeded; an output gate failed. Name the
        // failed gates — the script's own trailing lines are a
        // NOTE, not an error (this misreported once, live).
        let failed: Vec<String> = gates
            .iter()
            .filter(|g| g["result"].as_str() != Some("pass"))
            .map(|g| {
                format!(
                    "{}: {}",
                    g["gate"].as_str().unwrap_or("gate"),
                    g["detail"].as_str().unwrap_or("")
                )
            })
            .collect();
        Err(Failure::domain(
            "build_failed",
            format!(
                "build of {} ran clean but failed output gates: {}",
                project.id,
                failed.join("; ")
            ),
        ))
    } else {
        Err(Failure::domain(
            "build_failed",
            format!(
                "build of {} failed: {}",
                project.id,
                outcome.brief().lines().last().unwrap_or("see receipt")
            ),
        ))
    }
}

fn check_emulator_target(config: &Config, serial: &str) -> Result<(), Failure> {
    if !serial.starts_with("emulator-") {
        return Err(Failure::domain(
            "target_out_of_scope",
            format!(
                "target {serial} is not an emulator serial — installs in this build go to the local emulator only; physical devices are out of scope"
            ),
        ));
    }
    let devices = android::adb_devices(config).map_err(|e| Failure::domain("adb_failed", e))?;
    match devices.iter().find(|(s, _)| s == serial) {
        Some((_, state)) if state == "device" => Ok(()),
        Some((_, state)) => Err(Failure::domain(
            "target_not_ready",
            format!("target {serial} is in state {state}, not device"),
        )),
        None => Err(Failure::domain(
            "target_not_found",
            format!("target {serial} is not attached"),
        )),
    }
}

fn install(config: &Config, args: &Value) -> ToolResult {
    let project = project_of(config, args)?;
    let path = artifact_of(config, project, args)?;
    let serial = arg_str(args, "target")?;
    check_emulator_target(config, &serial)?;
    let hash = exec::sha256_file(&path)
        .map_err(|e| Failure::domain("io", format!("cannot hash artifact: {e}")))?;
    let outcome = android::adb(
        config,
        &serial,
        &["install", "-r", &path.to_string_lossy()],
        300,
    )
    .map_err(|e| Failure::domain("adb_failed", e))?;
    let mut record = base_receipt("install", project);
    record["output_hash"] = json!(hash);
    record["target"] = json!(serial);
    record["artifact"] = json!(path);
    let ok = outcome.ok() && outcome.brief().contains("Success");
    record["result"] = json!(if ok { "ok" } else { "failure" });
    record["gates"] = json!([{
        "gate": "adb_install", "result": if ok { "pass" } else { "fail" },
        "detail": outcome.brief().lines().last().unwrap_or("").to_string(),
    }]);
    receipt::append(&config.receipts_path(), record)?;
    if ok {
        Ok(
            json!({"project": project.id, "artifact": path, "sha256": hash, "target": serial, "verdict": "installed"}),
        )
    } else {
        Err(Failure::domain(
            "install_failed",
            format!("adb install failed: {}", outcome.brief()),
        ))
    }
}

fn device_open(config: &Config, args: &Value) -> ToolResult {
    let project = project_of(config, args)?;
    let serial = arg_str(args, "target")?;
    let hash = arg_str(args, "artifact_hash")?;
    check_emulator_target(config, &serial)?;
    // The artifact comes from slipway's own receipts, never a
    // free path (S2b session model).
    let receipts = receipt::read_all(&config.receipts_path());
    let found = receipts.iter().rev().find(|r| {
        r["output_hash"].as_str() == Some(hash.as_str())
            && r["project"].as_str() == Some(project.id.as_str())
    });
    let Some(found) = found else {
        return Err(Failure::domain(
            "artifact_not_in_receipts",
            "no receipt names this artifact hash for this project — device sessions open only on slipway-built artifacts",
        ));
    };
    let artifact_path = found["artifact"]
        .as_str()
        .or_else(|| found["outputs"][0]["path"].as_str())
        .ok_or_else(|| {
            Failure::domain(
                "artifact_not_in_receipts",
                "the receipt carries no artifact path",
            )
        })?
        .to_string();
    std::fs::create_dir_all(config.sessions_dir())
        .map_err(|e| Failure::domain("io", format!("cannot create sessions dir: {e}")))?;
    let session_id = format!(
        "{}-{}",
        project.id,
        &exec::sha256_bytes(format!("{serial}{hash}{}", exec::now_rfc3339()).as_bytes())[..12]
    );
    let session = json!({
        "session_id": session_id, "project": project.id, "target": serial,
        "artifact_hash": hash, "artifact": artifact_path,
        "package": project.package, "opened": exec::now_rfc3339(),
    });
    std::fs::write(
        config.sessions_dir().join(format!("{session_id}.json")),
        serde_json::to_string_pretty(&session).unwrap_or_default(),
    )
    .map_err(|e| Failure::domain("io", format!("cannot write session: {e}")))?;
    let mut record = base_receipt("device_open", project);
    record["output_hash"] = json!(hash);
    record["target"] = json!(serial);
    record["session"] = json!(session_id);
    receipt::append(&config.receipts_path(), record)?;
    Ok(session)
}

fn load_session(config: &Config, args: &Value) -> Result<Value, Failure> {
    let session_id = arg_str(args, "session_id")?;
    let path = config.sessions_dir().join(format!("{session_id}.json"));
    let text = std::fs::read_to_string(&path).map_err(|_| {
        Failure::domain("session_not_found", format!("no open session {session_id}"))
    })?;
    serde_json::from_str(&text)
        .map_err(|e| Failure::domain("session_not_found", format!("session unreadable: {e}")))
}

fn device_verify(config: &Config, args: &Value) -> ToolResult {
    let session = load_session(config, args)?;
    let project_id = session["project"].as_str().unwrap_or_default().to_string();
    let project = config.project(&project_id)?;
    let serial = session["target"].as_str().unwrap_or_default().to_string();
    let package = project.package.clone();
    let mut checks: Vec<Value> = Vec::new();
    // Installed?
    let pm = android::adb(config, &serial, &["shell", "pm", "path", &package], 60)
        .map_err(|e| Failure::domain("adb_failed", e))?;
    let installed = pm.ok() && pm.stdout.contains("package:");
    checks.push(json!({"check": "installed", "ok": installed, "detail": pm.stdout.trim()}));
    // Launch.
    let launch = android::adb(
        config,
        &serial,
        &[
            "shell",
            "monkey",
            "-p",
            &package,
            "-c",
            "android.intent.category.LAUNCHER",
            "1",
        ],
        60,
    )
    .map_err(|e| Failure::domain("adb_failed", e))?;
    checks.push(json!({"check": "launch_dispatched", "ok": launch.ok(), "detail": launch.brief().lines().last().unwrap_or("")}));
    std::thread::sleep(Duration::from_secs(3));
    // Process alive?
    let pid = android::adb(config, &serial, &["shell", "pidof", &package], 30)
        .map_err(|e| Failure::domain("adb_failed", e))?;
    let alive = pid.ok() && !pid.stdout.trim().is_empty();
    checks.push(json!({"check": "process_alive", "ok": alive, "detail": pid.stdout.trim()}));
    // Crash evidence in the recent log.
    let log = android::adb(config, &serial, &["logcat", "-d", "-t", "400"], 60)
        .map_err(|e| Failure::domain("adb_failed", e))?;
    // Crash evidence: a Java FATAL EXCEPTION naming the
    // process, or a native tombstone for the package
    // (`>>> pkg <<<` in the DEBUG header — a native crash logs
    // no FATAL EXCEPTION at all; the ONTrack release build's
    // native crash on the emulator taught this, live).
    let crashed = log.stdout.contains(&format!(">>> {package} <<<"))
        || (log.stdout.contains("FATAL EXCEPTION")
            && log.stdout.contains(&format!("Process: {package}")));
    checks.push(json!({"check": "no_fatal_crash", "ok": !crashed, "detail": if crashed { "FATAL EXCEPTION for the package in logcat" } else { "no fatal crash for the package in the log window" }}));
    let ok = checks.iter().all(|c| c["ok"].as_bool() == Some(true));
    let mut record = base_receipt("device_verify", project);
    record["output_hash"] = session["artifact_hash"].clone();
    record["target"] = json!(serial);
    record["session"] = session["session_id"].clone();
    record["result"] = json!(if ok { "ok" } else { "failure" });
    record["checks"] = json!(checks);
    receipt::append(&config.receipts_path(), record)?;
    Ok(json!({
        "session_id": session["session_id"], "package": package,
        "checks": checks, "verdict": if ok { "verified" } else { "failed" },
    }))
}

fn device_close(config: &Config, args: &Value) -> ToolResult {
    let session = load_session(config, args)?;
    let project_id = session["project"].as_str().unwrap_or_default().to_string();
    let project = config.project(&project_id)?;
    let session_id = session["session_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    std::fs::remove_file(config.sessions_dir().join(format!("{session_id}.json"))).ok();
    let mut record = base_receipt("device_close", project);
    record["session"] = json!(session_id);
    record["output_hash"] = session["artifact_hash"].clone();
    receipt::append(&config.receipts_path(), record)?;
    Ok(json!({"session_id": session_id, "verdict": "closed"}))
}

fn sign_tool(config: &Config, args: &Value) -> ToolResult {
    let project = project_of(config, args)?;
    let path = artifact_of(config, project, args)?;
    let manifest_value = args
        .get("manifest")
        .cloned()
        .ok_or_else(|| Failure::params("missing object argument: manifest"))?;
    let manifest: signing::SignManifest = serde_json::from_value(manifest_value)
        .map_err(|e| Failure::params(format!("manifest does not parse: {e}")))?;
    let outcome = signing::sign(config, project, &path, &manifest, exec::now_unix())?;
    let mut record = base_receipt("sign", project);
    record["actor"] = json!("owner-manifest via slipway");
    record["output_hash"] = outcome["output_hash"].clone();
    record["artifact"] = outcome["signed_artifact"].clone();
    record["unsigned"] = json!(false);
    record["channel"] = outcome["channel"].clone();
    receipt::append(&config.receipts_path(), record)?;
    Ok(outcome)
}

fn release_tool(config: &Config, args: &Value) -> ToolResult {
    let project = project_of(config, args)?;
    let channel = arg_str(args, "channel")?;
    let path = artifact_of(config, project, args)?;
    let cert_name = arg_str(args, "certification")?;
    let cert_path = if PathBuf::from(&cert_name).is_absolute() {
        PathBuf::from(&cert_name)
    } else {
        config.certifications_dir().join(&cert_name)
    };
    let bundle = release::assemble(config, project, &channel, &path, &cert_path)?;
    let mut record = base_receipt("release", project);
    if let Some(files) = bundle["manifest"]["files"].as_object()
        && let Some(artifact_file) = files
            .values()
            .find(|f| f["role"].as_str() == Some("artifact"))
    {
        record["output_hash"] = artifact_file["sha256"].clone();
    }
    record["bundle"] = bundle["bundle_dir"].clone();
    record["unsigned"] = json!(bundle["manifest"]["verdict"].as_str() != Some("certified"));
    receipt::append(&config.receipts_path(), record)?;
    Ok(bundle)
}

fn bundle_check(config: &Config, args: &Value) -> ToolResult {
    let name = arg_str(args, "bundle")?;
    let dir = release::resolve_bundle(config, &name)?;
    release::check(&dir)
}
