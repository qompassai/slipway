//! S4 release prep: assemble the submission bundle — the
//! deliverable. Slipway ends at the bundle (safety constitution
//! §3): no portal submission, no uploads, ever. A bundle carries
//! the artifact (by copy), its certification record, the
//! receipts behind it, release notes, and a bundle manifest
//! with hashes. `bundle_check` re-reads a bundle and verifies
//! completeness and hash integrity before it is presented.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::config::{Config, Project};
use crate::exec;
use crate::model::Failure;
use crate::receipt;

/// Assemble one release bundle for a project + channel from a
/// measured artifact and its certification record (the record
/// file must already exist — release prep never invents a
/// verdict). Returns the bundle directory and its manifest.
pub fn assemble(
    config: &Config,
    project: &Project,
    channel: &str,
    artifact: &Path,
    certification_record: &Path,
) -> Result<Value, Failure> {
    let canon = config.check_artifact_path(project, artifact)?;
    let cert_text = std::fs::read_to_string(certification_record).map_err(|e| {
        Failure::domain(
            "certification_missing",
            format!(
                "cannot read certification record {}: {e}",
                certification_record.display()
            ),
        )
    })?;
    let cert: Value = serde_json::from_str(&cert_text).map_err(|e| {
        Failure::domain(
            "certification_missing",
            format!("certification record does not parse: {e}"),
        )
    })?;
    if cert["project"].as_str() != Some(project.id.as_str())
        || cert["channel"].as_str() != Some(channel)
    {
        return Err(Failure::domain(
            "certification_mismatch",
            "certification record is for a different project or channel",
        ));
    }
    let artifact_hash = exec::sha256_file(&canon)
        .map_err(|e| Failure::domain("io", format!("cannot hash artifact: {e}")))?;
    if cert["artifact"]["sha256"].as_str() != Some(artifact_hash.as_str()) {
        return Err(Failure::domain(
            "certification_mismatch",
            "certification record is for a different artifact hash — refusing to bundle an uncertified artifact under a stale verdict",
        ));
    }
    let verdict = cert["verdict"].as_str().unwrap_or("unknown").to_string();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
    let bundle_dir = config
        .bundles_dir()
        .join(format!("{}-{}-{}", project.id, channel, stamp));
    std::fs::create_dir_all(&bundle_dir)
        .map_err(|e| Failure::domain("io", format!("cannot create bundle dir: {e}")))?;
    let artifact_name = canon
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_string());
    let copied = bundle_dir.join(&artifact_name);
    std::fs::copy(&canon, &copied)
        .map_err(|e| Failure::domain("io", format!("cannot copy artifact into bundle: {e}")))?;
    std::fs::copy(certification_record, bundle_dir.join("certification.json"))
        .map_err(|e| Failure::domain("io", format!("cannot copy certification: {e}")))?;
    // Receipts behind this artifact.
    let receipts = receipt::read_all(&config.receipts_path());
    let relevant: Vec<&Value> = receipts
        .iter()
        .filter(|r| r["project"].as_str() == Some(project.id.as_str()))
        .collect();
    let receipts_text = relevant
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        bundle_dir.join("receipts.jsonl"),
        format!("{receipts_text}\n"),
    )
    .map_err(|e| Failure::domain("io", format!("cannot write bundle receipts: {e}")))?;
    let notes = format!(
        "# Release bundle — {} ({})\n\n- Artifact: {} (sha256 {})\n- Certification verdict: {}\n- Prepared: {}\n\nSubmission is the Owner's act in the channel portal. This bundle is the deliverable; slipway submits nothing.\n",
        project.name,
        channel,
        artifact_name,
        artifact_hash,
        verdict,
        exec::now_rfc3339()
    );
    std::fs::write(bundle_dir.join("RELEASE-NOTES.md"), &notes)
        .map_err(|e| Failure::domain("io", format!("cannot write release notes: {e}")))?;
    let manifest = json!({
        "schema": "slipway.bundle/v1",
        "project": project.id,
        "channel": channel,
        "prepared": exec::now_rfc3339(),
        "verdict": verdict,
        "files": {
            artifact_name.clone(): {"sha256": artifact_hash, "role": "artifact"},
            "certification.json": {"sha256": exec::sha256_bytes(cert_text.as_bytes()), "role": "certification"},
            "receipts.jsonl": {"sha256": exec::sha256_bytes(format!("{receipts_text}\n").as_bytes()), "role": "receipts"},
            "RELEASE-NOTES.md": {"sha256": exec::sha256_bytes(notes.as_bytes()), "role": "notes"},
        },
        "owner_stamp_required": verdict != "certified",
    });
    std::fs::write(
        bundle_dir.join("bundle.json"),
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    )
    .map_err(|e| Failure::domain("io", format!("cannot write bundle manifest: {e}")))?;
    Ok(json!({
        "bundle_dir": bundle_dir,
        "manifest": manifest,
    }))
}

/// Verify a bundle: manifest parses, every listed file exists
/// and hashes to its recorded value, the artifact's hash still
/// matches the certification record inside the bundle.
pub fn check(bundle_dir: &Path) -> Result<Value, Failure> {
    let manifest_path = bundle_dir.join("bundle.json");
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| {
        Failure::domain(
            "bundle_invalid",
            format!("cannot read {}: {e}", manifest_path.display()),
        )
    })?;
    let manifest: Value = serde_json::from_str(&text).map_err(|e| {
        Failure::domain(
            "bundle_invalid",
            format!("bundle manifest does not parse: {e}"),
        )
    })?;
    let mut checked = Vec::new();
    let mut problems = Vec::new();
    if let Some(files) = manifest["files"].as_object() {
        for (name, entry) in files {
            let path = bundle_dir.join(name);
            match exec::sha256_file(&path) {
                Ok(hash) => {
                    let recorded = entry["sha256"].as_str().unwrap_or("");
                    if hash == recorded {
                        checked.push(name.clone());
                    } else {
                        problems.push(format!(
                            "{name}: hash mismatch (file {hash}, manifest {recorded})"
                        ));
                    }
                }
                Err(_) => problems.push(format!("{name}: missing from bundle")),
            }
        }
    }
    // Artifact ↔ certification agreement inside the bundle.
    if let Ok(cert_text) = std::fs::read_to_string(bundle_dir.join("certification.json")) {
        if let Ok(cert) = serde_json::from_str::<Value>(&cert_text)
            && let Some(files) = manifest["files"].as_object()
        {
            for (name, entry) in files {
                if entry["role"].as_str() == Some("artifact")
                    && cert["artifact"]["sha256"].as_str() != entry["sha256"].as_str()
                {
                    problems.push(format!(
                        "{name}: certification record names a different artifact hash"
                    ));
                }
            }
        }
    } else {
        problems.push("certification.json missing or unreadable".to_string());
    }
    Ok(json!({
        "bundle_dir": bundle_dir,
        "files_checked": checked,
        "problems": problems,
        "valid": problems.is_empty(),
        "verdict": manifest["verdict"],
    }))
}

/// Locate a bundle by directory path under the bundles dir.
pub fn resolve_bundle(config: &Config, name: &str) -> Result<PathBuf, Failure> {
    let candidate = config.bundles_dir().join(name);
    let canon = candidate
        .canonicalize()
        .map_err(|e| Failure::domain("bundle_invalid", format!("bundle {name} not found: {e}")))?;
    if !canon.starts_with(config.bundles_dir()) {
        return Err(Failure::domain(
            "bundle_invalid",
            "bundle path escapes the bundles dir",
        ));
    }
    Ok(canon)
}
