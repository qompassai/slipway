//! S3 signing machinery — manifest-gated, from day one (Q7).
//!
//! The design (STUDY.md key-custody section): keys never live in
//! the server — a keystore is referenced by path only, and its
//! password reaches the signer binary through a named
//! environment variable, never through slipway's memory, logs,
//! receipts, or tool results. One canonical signed manifest per
//! signing action: project, artifact hash, channel, key
//! reference, nonce, issue time — signed by the action-signing
//! key whose public half is configured as trusted. Verification
//! is fail-closed and freshness-bounded (release-class ≤ 60 s);
//! the nonce is consumed BEFORE dispatch, so a crashed or
//! refused dispatch cannot be replayed.
//!
//! In this build the machinery is complete and proven against a
//! labelled throwaway test key. No real keystore is configured
//! and no real artifact is signed: that act is the Owner's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::{Config, Project};
use crate::exec;
use crate::model::Failure;
use crate::receipt;

/// Release-class manifest freshness bound (seconds), fail-closed.
pub const FRESHNESS_SECS: i64 = 60;
/// Tolerated issuer clock skew (seconds).
pub const SKEW_SECS: i64 = 5;

/// One signed signing manifest (the Owner's stamp).
#[derive(Debug, Clone, Deserialize)]
pub struct SignManifest {
    pub schema: String,
    pub project: String,
    pub artifact_hash: String,
    pub channel: String,
    pub key_ref: String,
    pub nonce: String,
    pub issued_unix: i64,
    /// Hex ed25519 signature over [`canonical`].
    pub signature: String,
}

impl SignManifest {
    /// The canonical byte string the signature covers.
    pub fn canonical(&self) -> String {
        format!(
            "slipway.sign-manifest/v1\n{}\n{}\n{}\n{}\n{}\n{}\n",
            self.project,
            self.artifact_hash,
            self.channel,
            self.key_ref,
            self.nonce,
            self.issued_unix
        )
    }
}

fn hex_bytes(text: &str) -> Result<Vec<u8>, Failure> {
    if !text.len().is_multiple_of(2) {
        return Err(Failure::domain(
            "manifest_invalid",
            "hex field has odd length",
        ));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&text[i..i + 2], 16)
                .map_err(|_| Failure::domain("manifest_invalid", "hex field is not hex"))
        })
        .collect()
}

/// Verify a manifest's signature against the trusted key set.
pub fn verify_signature(
    manifest: &SignManifest,
    trusted_pubkeys_hex: &[String],
) -> Result<(), Failure> {
    if manifest.schema != "slipway.sign-manifest/v1" {
        return Err(Failure::domain(
            "manifest_invalid",
            format!("unknown manifest schema: {}", manifest.schema),
        ));
    }
    let signature_bytes = hex_bytes(&manifest.signature)?;
    let signature = Signature::from_slice(&signature_bytes).map_err(|_| {
        Failure::domain(
            "manifest_invalid",
            "signature is not a 64-byte ed25519 signature",
        )
    })?;
    for key_hex in trusted_pubkeys_hex {
        let Ok(key_bytes) = hex_bytes(key_hex) else {
            continue;
        };
        let Ok(array) = <[u8; 32]>::try_from(key_bytes.as_slice()) else {
            continue;
        };
        let Ok(key) = VerifyingKey::from_bytes(&array) else {
            continue;
        };
        if key
            .verify(manifest.canonical().as_bytes(), &signature)
            .is_ok()
        {
            return Ok(());
        }
    }
    Err(Failure::domain(
        "manifest_signature_refused",
        "manifest signature does not verify against any trusted action-signing key",
    ))
}

/// Freshness, fail-closed both directions.
pub fn check_freshness(manifest: &SignManifest, now_unix: i64) -> Result<(), Failure> {
    let age = now_unix - manifest.issued_unix;
    if age < -SKEW_SECS {
        return Err(Failure::domain(
            "manifest_from_future",
            format!(
                "manifest issued {}s in the future (skew bound {SKEW_SECS}s)",
                -age
            ),
        ));
    }
    if age > FRESHNESS_SECS {
        return Err(Failure::domain(
            "manifest_stale",
            format!("manifest is {age}s old; release-class freshness bound is {FRESHNESS_SECS}s"),
        ));
    }
    Ok(())
}

fn load_nonces(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default()
}

/// Consume a nonce: record it before dispatch. Replays refuse.
pub fn consume_nonce(path: &Path, nonce: &str) -> Result<(), Failure> {
    let mut nonces = load_nonces(path);
    if nonces.iter().any(|n| n == nonce) {
        return Err(Failure::domain(
            "nonce_replayed",
            "manifest nonce was already consumed — replay refused",
        ));
    }
    nonces.push(nonce.to_string());
    let text = serde_json::to_string(&nonces)
        .map_err(|e| Failure::domain("nonce_store_failed", format!("nonce encode failed: {e}")))?;
    std::fs::write(path, text)
        .map_err(|e| Failure::domain("nonce_store_failed", format!("nonce persist failed: {e}")))?;
    Ok(())
}

/// The full gated signing dispatch. Every check runs before the
/// signer binary is invoked; the nonce is consumed before
/// dispatch; the signed output is verified before its hash is
/// returned as release-eligible.
pub fn sign(
    config: &Config,
    project: &Project,
    artifact: &Path,
    manifest: &SignManifest,
    now_unix: i64,
) -> Result<Value, Failure> {
    // Gate 1: the manifest names this project and this key.
    if manifest.project != project.id {
        return Err(Failure::domain(
            "manifest_project_mismatch",
            format!(
                "manifest is for project {}, not {}",
                manifest.project, project.id
            ),
        ));
    }
    let configured_ref = project.keystore_ref.clone().ok_or_else(|| {
        Failure::domain(
            "key_unavailable",
            format!(
                "project {} has no keystore reference configured",
                project.id
            ),
        )
    })?;
    if manifest.key_ref != configured_ref {
        return Err(Failure::domain(
            "manifest_key_mismatch",
            "manifest key reference does not match the project's configured key reference",
        ));
    }
    if !Path::new(&configured_ref).is_file() {
        return Err(Failure::domain(
            "key_unavailable",
            format!(
                "keystore reference {configured_ref} does not resolve to a file — secret unavailable (structured, fail-closed)"
            ),
        ));
    }
    let pass_env = project.keystore_pass_env.clone().ok_or_else(|| {
        Failure::domain(
            "key_unavailable",
            "project has no keystore password env reference",
        )
    })?;
    if std::env::var(&pass_env).is_err() {
        return Err(Failure::domain(
            "key_unavailable",
            format!("password environment variable {pass_env} is not set — secret unavailable"),
        ));
    }
    // Gate 2: signature + freshness.
    verify_signature(manifest, &config.signer_pubkeys)?;
    check_freshness(manifest, now_unix)?;
    // Gate 3: the artifact is the manifest's artifact, inside
    // the project root, and not gate-ineligible.
    let canon = config.check_artifact_path(project, artifact)?;
    let hash = exec::sha256_file(&canon)
        .map_err(|e| Failure::domain("io", format!("cannot hash artifact: {e}")))?;
    if hash != manifest.artifact_hash {
        return Err(Failure::domain(
            "manifest_hash_mismatch",
            "artifact hash does not match the manifest — refused",
        ));
    }
    if receipt::is_ineligible(&config.receipts_path(), &hash) {
        return Err(Failure::domain(
            "artifact_ineligible",
            "a failed gate has marked this artifact hash ineligible for signing",
        ));
    }
    // Gate 4: consume the nonce BEFORE dispatch.
    consume_nonce(&config.nonces_path(), &manifest.nonce)?;
    // Dispatch: the signer binary reads the keystore itself; the
    // password travels only via the named environment variable.
    let file_name = canon
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let is_aab = file_name.ends_with(".aab");
    let out_name = file_name
        .replace(".apk", ".signed.apk")
        .replace(".aab", ".signed.aab");
    let out_path: PathBuf = canon.with_file_name(out_name);
    let mut env = BTreeMap::new();
    if let Ok(value) = std::env::var(&pass_env) {
        env.insert(pass_env.clone(), value);
    }
    let alias = project
        .key_alias
        .clone()
        .unwrap_or_else(|| "key".to_string());
    let outcome = if is_aab {
        let Some(jarsigner) = config.program("jarsigner") else {
            return Err(Failure::domain(
                "key_unavailable",
                "jarsigner not available",
            ));
        };
        exec::run(
            &[
                jarsigner.to_string_lossy().into_owned(),
                "-keystore".to_string(),
                configured_ref.clone(),
                "-storepass:env".to_string(),
                pass_env.clone(),
                "-signedjar".to_string(),
                out_path.to_string_lossy().into_owned(),
                canon.to_string_lossy().into_owned(),
                alias,
            ],
            None,
            &env,
            Duration::from_secs(120),
        )
    } else {
        let Some(apksigner) = config.program("apksigner") else {
            return Err(Failure::domain(
                "key_unavailable",
                "apksigner not available",
            ));
        };
        exec::run(
            &[
                apksigner.to_string_lossy().into_owned(),
                "sign".to_string(),
                "--ks".to_string(),
                configured_ref.clone(),
                "--ks-key-alias".to_string(),
                alias,
                "--ks-pass".to_string(),
                format!("env:{pass_env}"),
                "--key-pass".to_string(),
                format!("env:{pass_env}"),
                "--out".to_string(),
                out_path.to_string_lossy().into_owned(),
                canon.to_string_lossy().into_owned(),
            ],
            None,
            &env,
            Duration::from_secs(120),
        )
    };
    if !outcome.ok() {
        return Err(Failure::domain(
            "sign_failed",
            format!("signer exited {:?}: {}", outcome.exit_code, outcome.brief()),
        ));
    }
    // Gate 5: the signed output verifies before it is eligible.
    let facts = crate::android::signature_facts(config, &out_path);
    if facts.state != "signed" {
        return Err(Failure::domain(
            "sign_unverified",
            format!("signed output does not verify: {}", facts.detail),
        ));
    }
    let out_hash = exec::sha256_file(&out_path)
        .map_err(|e| Failure::domain("io", format!("cannot hash signed output: {e}")))?;
    Ok(json!({
        "signed_artifact": out_path,
        "output_hash": out_hash,
        "signer_fingerprint": facts.signer_sha256,
        "channel": manifest.channel,
        "nonce_consumed": manifest.nonce,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;

    fn test_key() -> (ed25519_dalek::SigningKey, String) {
        // Labelled throwaway test key: fixed seed, tests only.
        let key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pub_hex = key
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        (key, pub_hex)
    }

    fn signed_manifest(key: &ed25519_dalek::SigningKey, issued: i64, nonce: &str) -> SignManifest {
        let mut manifest = SignManifest {
            schema: "slipway.sign-manifest/v1".to_string(),
            project: "x".to_string(),
            artifact_hash: "ab".repeat(32),
            channel: "google-play".to_string(),
            key_ref: "/tmp/throwaway.jks".to_string(),
            nonce: nonce.to_string(),
            issued_unix: issued,
            signature: String::new(),
        };
        let signature = key.sign(manifest.canonical().as_bytes());
        manifest.signature = signature
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        manifest
    }

    #[test]
    fn valid_manifest_verifies() {
        let (key, pub_hex) = test_key();
        let manifest = signed_manifest(&key, 1_000, "n1");
        assert!(verify_signature(&manifest, &[pub_hex]).is_ok());
    }

    #[test]
    fn tampered_manifest_is_refused() {
        let (key, pub_hex) = test_key();
        let mut manifest = signed_manifest(&key, 1_000, "n1");
        manifest.artifact_hash = "cd".repeat(32);
        match verify_signature(&manifest, &[pub_hex]) {
            Err(f) => assert_eq!(f.code, "manifest_signature_refused"),
            Ok(()) => panic!("tampered manifest must be refused"),
        }
    }

    #[test]
    fn stale_and_future_manifests_are_refused() {
        let (_key, _pub) = test_key();
        let manifest = signed_manifest(&_key, 1_000, "n1");
        assert_eq!(
            check_freshness(&manifest, 1_000 + FRESHNESS_SECS + 1)
                .expect_err("stale")
                .code,
            "manifest_stale"
        );
        assert_eq!(
            check_freshness(&manifest, 1_000 - SKEW_SECS - 1)
                .expect_err("future")
                .code,
            "manifest_from_future"
        );
        assert!(check_freshness(&manifest, 1_000 + FRESHNESS_SECS).is_ok());
    }

    #[test]
    fn nonce_replay_is_refused() {
        let tmp = std::env::temp_dir().join(format!("slipway-nonce-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("mkdir");
        let path = tmp.join("nonces.json");
        assert!(consume_nonce(&path, "nonce-1").is_ok());
        match consume_nonce(&path, "nonce-1") {
            Err(f) => assert_eq!(f.code, "nonce_replayed"),
            Ok(()) => panic!("replayed nonce must be refused"),
        }
        assert!(consume_nonce(&path, "nonce-2").is_ok());
        std::fs::remove_dir_all(&tmp).ok();
    }
}
