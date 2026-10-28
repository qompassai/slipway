//! The Android toolchain surface: presence/version probes,
//! manifest read-back from the artifact (aapt2 for APKs,
//! bundletool for AABs), signature verification (apksigner /
//! jarsigner), and the adb verbs. Everything shells through
//! [`crate::exec`] — bounded, capped, receipted by callers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::config::Config;
use crate::exec::{self, Outcome};

fn env() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn run1(config: &Config, program: &str, args: &[&str], timeout_secs: u64) -> Option<Outcome> {
    let path = config.program(program)?;
    let mut argv = vec![path.to_string_lossy().into_owned()];
    argv.extend(args.iter().map(|s| s.to_string()));
    Some(exec::run(
        &argv,
        None,
        &env(),
        Duration::from_secs(timeout_secs),
    ))
}

/// Probe one program: resolved path + `--version`-class output.
fn probe(config: &Config, program: &str, version_args: &[&str]) -> Value {
    match config.program(program) {
        None => json!({"program": program, "present": false}),
        Some(path) => {
            let mut argv = vec![path.to_string_lossy().into_owned()];
            argv.extend(version_args.iter().map(|s| s.to_string()));
            let outcome = exec::run(&argv, None, &env(), Duration::from_secs(30));
            let first_line = outcome
                .stdout
                .lines()
                .chain(outcome.stderr.lines())
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            json!({
                "program": program,
                "present": true,
                "path": path,
                "version_line": first_line,
            })
        }
    }
}

/// The toolchain census `slipway_toolchain` reports. Absences
/// are facts, not failures.
pub fn toolchain_census(config: &Config) -> Value {
    let probes = [
        probe(config, "java", &["-version"]),
        probe(config, "adb", &["version"]),
        probe(config, "apksigner", &["version"]),
        probe(config, "aapt2", &["version"]),
        probe(config, "bundletool", &["version"]),
        probe(config, "jarsigner", &["-help"]),
        probe(config, "keytool", &["-help"]),
        probe(config, "readelf", &["--version"]),
        probe(config, "emulator", &["-version"]),
    ];
    let java_home = std::env::var("JAVA_HOME").ok();
    let android_home = std::env::var("ANDROID_HOME").ok();
    json!({
        "probes": probes,
        "java_home": java_home,
        "android_home": android_home,
    })
}

/// Manifest facts read back from the artifact itself.
pub struct ManifestFacts {
    pub package: Option<String>,
    pub version_code: Option<i64>,
    pub version_name: Option<String>,
    pub min_sdk: Option<i64>,
    pub target_sdk: Option<i64>,
    pub compile_sdk: Option<i64>,
    pub permissions: Vec<String>,
    pub source: String,
}

fn field(text: &str, key: &str) -> Option<String> {
    // aapt2 badging: key:'value' ; bundletool: key="value"
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key) {
            let rest = rest.strip_prefix(':').unwrap_or(rest).trim();
            let rest = rest.strip_prefix('=').unwrap_or(rest).trim();
            let value = rest.trim_matches(|c| c == '\'' || c == '"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Read the manifest of an APK (aapt2) or AAB (bundletool).
pub fn manifest_facts(config: &Config, artifact: &Path) -> Result<ManifestFacts, String> {
    let name = artifact
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.ends_with(".aab") {
        let outcome = run1(
            config,
            "bundletool",
            &["dump", "manifest", "--bundle", &artifact.to_string_lossy()],
            60,
        )
        .ok_or("bundletool not available")?;
        if !outcome.ok() {
            return Err(format!(
                "bundletool dump manifest failed: {}",
                outcome.brief()
            ));
        }
        let text = &outcome.stdout;
        let attr = |key: &str| -> Option<String> {
            // bundletool XML: android:targetSdkVersion="36"
            text.lines().find_map(|line| {
                line.split(&format!("{key}=\""))
                    .nth(1)
                    .and_then(|rest| rest.split('"').next())
                    .map(|s| s.to_string())
            })
        };
        let permissions = text
            .lines()
            .filter_map(|line| {
                line.split("uses-permission")
                    .nth(1)
                    .and_then(|_| line.split("android:name=\"").nth(1))
                    .and_then(|rest| rest.split('"').next())
                    .map(|s| s.to_string())
            })
            .collect();
        return Ok(ManifestFacts {
            package: attr("package"),
            version_code: attr("versionCode").and_then(|v| v.parse().ok()),
            version_name: attr("versionName"),
            min_sdk: attr("minSdkVersion").and_then(|v| v.parse().ok()),
            target_sdk: attr("targetSdkVersion").and_then(|v| v.parse().ok()),
            compile_sdk: attr("compileSdkVersion").and_then(|v| v.parse().ok()),
            permissions,
            source: "bundletool dump manifest".to_string(),
        });
    }
    let outcome = run1(
        config,
        "aapt2",
        &["dump", "badging", &artifact.to_string_lossy()],
        60,
    )
    .ok_or("aapt2 not available")?;
    if !outcome.ok() {
        return Err(format!("aapt2 dump badging failed: {}", outcome.brief()));
    }
    let text = &outcome.stdout;
    let package_line = text
        .lines()
        .next()
        .unwrap_or("")
        .trim_start_matches("package:");
    let pick = |src: &str, key: &str| -> Option<String> {
        src.split(&format!("{key}='"))
            .nth(1)
            .and_then(|rest| rest.split('\'').next())
            .map(|s| s.to_string())
    };
    let permissions = text
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("uses-permission:")
                .and_then(|rest| rest.trim().strip_prefix("name='"))
                .and_then(|rest| rest.split('\'').next())
                .map(|s| s.to_string())
        })
        .collect();
    Ok(ManifestFacts {
        package: pick(package_line, "name"),
        version_code: pick(package_line, "versionCode").and_then(|v| v.parse().ok()),
        version_name: pick(package_line, "versionName"),
        min_sdk: field(text, "sdkVersion").and_then(|v| v.parse().ok()),
        target_sdk: field(text, "targetSdkVersion").and_then(|v| v.parse().ok()),
        compile_sdk: None,
        permissions,
        source: "aapt2 dump badging".to_string(),
    })
}

impl ManifestFacts {
    pub fn to_json(&self) -> Value {
        json!({
            "package": self.package,
            "version_code": self.version_code,
            "version_name": self.version_name,
            "min_sdk": self.min_sdk,
            "target_sdk": self.target_sdk,
            "compile_sdk": self.compile_sdk,
            "permissions": self.permissions,
            "read_via": self.source,
        })
    }
}

/// Signature state read back from the artifact.
pub struct SignatureFacts {
    /// `signed` | `unsigned` | `unverified` (tool missing/failed)
    pub state: String,
    pub signer_sha256: Option<String>,
    pub detail: String,
}

pub fn signature_facts(config: &Config, artifact: &Path) -> SignatureFacts {
    let name = artifact
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.ends_with(".aab") {
        // jarsigner exits 0 on UNSIGNED jars too (it prints
        // "jar is unsigned" / "no manifest."), so the exit code
        // proves nothing: the signed state requires the positive
        // "jar verified" marker and the absence of both unsigned
        // markers. (Caught live: the first certification run
        // reported the by-design-unsigned LightShow AAB as
        // signed on the strength of exit 0 alone.)
        return match run1(
            config,
            "jarsigner",
            &["-verify", &artifact.to_string_lossy()],
            60,
        ) {
            None => SignatureFacts {
                state: "unverified".to_string(),
                signer_sha256: None,
                detail: "jarsigner not available".to_string(),
            },
            Some(outcome) => {
                let text = outcome.brief();
                let unsigned_marker =
                    text.contains("jar is unsigned") || text.contains("no manifest");
                if outcome.ok() && text.contains("jar verified") && !unsigned_marker {
                    SignatureFacts {
                        state: "signed".to_string(),
                        signer_sha256: None,
                        detail: "jarsigner -verify: jar verified".to_string(),
                    }
                } else if unsigned_marker {
                    SignatureFacts {
                        state: "unsigned".to_string(),
                        signer_sha256: None,
                        detail: format!(
                            "jarsigner -verify: {}",
                            text.lines()
                                .find(|l| l.contains("unsigned") || l.contains("no manifest"))
                                .unwrap_or("unsigned")
                        ),
                    }
                } else {
                    SignatureFacts {
                        state: "unverified".to_string(),
                        signer_sha256: None,
                        detail: format!(
                            "jarsigner -verify exit {:?}: {}",
                            outcome.exit_code,
                            text.lines().next().unwrap_or("")
                        ),
                    }
                }
            }
        };
    }
    match run1(
        config,
        "apksigner",
        &["verify", "--print-certs", &artifact.to_string_lossy()],
        60,
    ) {
        None => SignatureFacts {
            state: "unverified".to_string(),
            signer_sha256: None,
            detail: "apksigner not available".to_string(),
        },
        Some(outcome) if outcome.ok() => {
            let fingerprint = outcome.stdout.lines().find_map(|line| {
                line.split("SHA-256 digest:")
                    .nth(1)
                    .map(|s| s.trim().replace(':', "").to_lowercase())
            });
            SignatureFacts {
                state: "signed".to_string(),
                signer_sha256: fingerprint,
                detail: "apksigner verify: OK".to_string(),
            }
        }
        Some(outcome) => SignatureFacts {
            state: "unsigned".to_string(),
            signer_sha256: None,
            detail: format!(
                "apksigner verify exit {:?}: {}",
                outcome.exit_code,
                outcome.brief().lines().next().unwrap_or("")
            ),
        },
    }
}

/// `adb devices` parsed to (serial, state) pairs.
pub fn adb_devices(config: &Config) -> Result<Vec<(String, String)>, String> {
    let outcome = run1(config, "adb", &["devices"], 30).ok_or("adb not available")?;
    if !outcome.ok() {
        return Err(format!("adb devices failed: {}", outcome.brief()));
    }
    Ok(outcome
        .stdout
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some((parts.next()?.to_string(), parts.next()?.to_string()))
        })
        .collect())
}

/// Run an adb command against one named serial.
pub fn adb(
    config: &Config,
    serial: &str,
    args: &[&str],
    timeout_secs: u64,
) -> Result<Outcome, String> {
    let path: PathBuf = config.program("adb").ok_or("adb not available")?;
    let mut argv = vec![
        path.to_string_lossy().into_owned(),
        "-s".to_string(),
        serial.to_string(),
    ];
    argv.extend(args.iter().map(|s| s.to_string()));
    Ok(exec::run(
        &argv,
        None,
        &env(),
        Duration::from_secs(timeout_secs),
    ))
}
