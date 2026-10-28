//! D-121 publish certification: slipway EXECUTES the gate
//! batteries the publish skills own (local-light-show-publish,
//! local-ontrack-publish, local-fdroid-publish) and issues a
//! per-app, per-channel verdict with measured evidence. The
//! gate definitions live in those skills; this module is the
//! executor, never a fork of the definitions — every gate below
//! names the skill rule it executes.
//!
//! Gate results: `pass` (measured, meets the rule), `fail`
//! (measured, breaks the rule), `owner_action` (the rule's act
//! belongs to the Owner — signing, Console identity acts),
//! `conditional` (the rule applies only under a condition that
//! is not on record), `unverified` (could not be measured here).
//! A channel is `certified` only when every gate passes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::android;
use crate::artifact;
use crate::config::{Config, Project};
use crate::exec;
use crate::model::Failure;

pub struct Gate {
    pub name: String,
    pub result: String,
    pub detail: String,
    pub evidence: Value,
}

impl Gate {
    fn new(name: &str, result: &str, detail: impl Into<String>) -> Gate {
        Gate {
            name: name.to_string(),
            result: result.to_string(),
            detail: detail.into(),
            evidence: json!({}),
        }
    }
    fn with_evidence(mut self, evidence: Value) -> Gate {
        self.evidence = evidence;
        self
    }
    fn to_json(&self) -> Value {
        json!({
            "gate": self.name,
            "result": self.result,
            "detail": self.detail,
            "evidence": self.evidence,
        })
    }
}

pub struct Certification {
    pub project: String,
    pub channel: String,
    pub issued: String,
    pub artifact: Value,
    pub gates: Vec<Gate>,
}

impl Certification {
    pub fn verdict(&self) -> &'static str {
        if self.gates.iter().all(|g| g.result == "pass") {
            "certified"
        } else {
            "not_certified"
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "schema": "slipway.certification/v1",
            "project": self.project,
            "channel": self.channel,
            "issued": self.issued,
            "artifact": self.artifact,
            "gates": self.gates.iter().map(|g| g.to_json()).collect::<Vec<_>>(),
            "verdict": self.verdict(),
            "blocking": self.gates.iter()
                .filter(|g| g.result != "pass")
                .map(|g| json!({"gate": g.name, "result": g.result, "detail": g.detail}))
                .collect::<Vec<_>>(),
        })
    }
}

struct Ctx {
    census: artifact::Census,
    manifest: Result<android::ManifestFacts, String>,
    signature: android::SignatureFacts,
}

fn scan_listing_images(_project: &Project, dirs: &[PathBuf]) -> Gate {
    let mut checked: Vec<Value> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut stack: Vec<PathBuf> = dirs.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !(name.ends_with(".png") || name.ends_with(".jpg") || name.ends_with(".jpeg")) {
                continue;
            }
            if name.contains("icon") {
                checked.push(json!({"file": path, "disposition": "icon — exempt (32-bit with alpha is the icon spec)"}));
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                failures.push(format!("{}: unreadable", path.display()));
                continue;
            };
            if artifact::is_jpeg(&bytes) {
                checked.push(json!({"file": path, "format": "jpeg", "result": "pass"}));
            } else if let Some(facts) = artifact::png_facts(&bytes) {
                let ok = facts.is_24bit_no_alpha();
                if !ok {
                    failures.push(format!(
                        "{}: PNG color type {} bit depth {} — not 24-bit/no-alpha",
                        path.display(),
                        facts.color_type,
                        facts.bit_depth
                    ));
                }
                checked.push(json!({
                    "file": path, "format": "png",
                    "width": facts.width, "height": facts.height,
                    "color_type": facts.color_type, "bit_depth": facts.bit_depth,
                    "result": if ok { "pass" } else { "fail" },
                }));
            } else {
                failures.push(format!("{}: not a readable PNG/JPEG", path.display()));
            }
        }
    }
    if checked.is_empty() && failures.is_empty() {
        return Gate::new(
            "listing_images_no_alpha",
            "unverified",
            "no listing images found in the configured listing trees",
        );
    }
    if failures.is_empty() {
        Gate::new(
            "listing_images_no_alpha",
            "pass",
            format!(
                "{} listing images measured 24-bit/no-alpha or JPEG (icons exempt)",
                checked.len()
            ),
        )
        .with_evidence(json!({"images": checked}))
    } else {
        Gate::new(
            "listing_images_no_alpha",
            "fail",
            format!(
                "{} listing image(s) break the no-alpha rule",
                failures.len()
            ),
        )
        .with_evidence(json!({"failures": failures, "images": checked}))
    }
}

fn listing_dirs(config: &Config, project: &Project) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(listing) = &project.listing_dir
        && let Ok(p) = config.project_path(project, listing)
    {
        dirs.push(p);
    }
    for extra in &project.extra_listing_dirs {
        if let Ok(p) = config.project_path(project, extra) {
            dirs.push(p);
        }
    }
    dirs
}

/// Gates shared by both channels (artifact-measured rules).
fn artifact_gates(ctx: &Ctx) -> Vec<Gate> {
    let mut gates = Vec::new();
    match &ctx.manifest {
        Ok(facts) => {
            gates.push(
                Gate::new(
                    "manifest_readback",
                    "pass",
                    format!("manifest read back from the artifact via {}", facts.source),
                )
                .with_evidence(facts.to_json()),
            );
            let target = facts.target_sdk;
            gates.push(
                Gate::new(
                    "target_sdk_36",
                    if target == Some(36) { "pass" } else { "fail" },
                    format!("targetSdkVersion read back as {target:?}; the skills require 36"),
                )
                .with_evidence(json!({"target_sdk": target})),
            );
        }
        Err(e) => {
            gates.push(Gate::new(
                "manifest_readback",
                "fail",
                format!("manifest could not be read from the artifact: {e}"),
            ));
            gates.push(Gate::new(
                "target_sdk_36",
                "unverified",
                "no manifest read-back",
            ));
        }
    }
    let table = artifact::alignment_table(&ctx.census);
    if ctx.census.native_libs.is_empty() {
        gates.push(Gate::new(
            "alignment_16k",
            "pass",
            "no native libraries in the artifact; the 16 KB rule does not apply",
        ));
    } else if table["libs_64bit"].as_u64() == Some(0) {
        gates.push(
            Gate::new(
                "alignment_16k",
                "pass",
                "only 32-bit native libraries; the 16 KB rule scopes to 64-bit",
            )
            .with_evidence(table),
        );
    } else if table["all_64bit_meet_16k"].as_bool() == Some(true) {
        gates.push(
            Gate::new(
                "alignment_16k",
                "pass",
                format!(
                    "all {} 64-bit .so LOAD alignments are 0x4000",
                    table["libs_64bit"]
                ),
            )
            .with_evidence(table),
        );
    } else {
        gates.push(
            Gate::new(
                "alignment_16k",
                "fail",
                "at least one 64-bit .so has LOAD alignment below 0x4000 (Play blocks updates from 2027-02-01)",
            )
            .with_evidence(table),
        );
    }
    gates
}

fn developer_verification_gate(project: &Project) -> Gate {
    if project.developer_verification_registered {
        Gate::new(
            "developer_verification",
            "pass",
            "package + signing key stand registered under the Owner's verified developer identity (on record)",
        )
    } else {
        Gate::new(
            "developer_verification",
            "owner_action",
            "package + signing key must be registered under the Owner's verified developer identity before the 2027 enforcement window — an Owner identity act, not on record",
        )
    }
}

fn play_battery(config: &Config, project: &Project, ctx: &Ctx) -> Vec<Gate> {
    let mut gates = artifact_gates(ctx);
    // Package identity vs config.
    if let Ok(facts) = &ctx.manifest {
        let measured = facts.package.clone().unwrap_or_default();
        gates.push(Gate::new(
            "package_identity",
            if measured == project.package {
                "pass"
            } else {
                "fail"
            },
            format!(
                "manifest package {measured}; configured package {}",
                project.package
            ),
        ));
    }
    // Signature state (Play upload needs a signed artifact).
    gates.push(match ctx.signature.state.as_str() {
        "signed" => Gate::new(
            "signature_state",
            "pass",
            format!("artifact is signed ({})", ctx.signature.detail),
        ),
        "unsigned" => Gate::new(
            "signature_state",
            "owner_action",
            format!(
                "artifact is unsigned ({}); signing is the Owner's step in this project's pipeline",
                ctx.signature.detail
            ),
        ),
        _ => Gate::new(
            "signature_state",
            "unverified",
            format!(
                "signature state could not be measured: {}",
                ctx.signature.detail
            ),
        ),
    });
    // Listing images.
    gates.push(scan_listing_images(project, &listing_dirs(config, project)));
    // Data Safety consistency (per-app rules from the skills:
    // light-show offline → zero permissions proves "no data";
    // ONTrack networked → declared data in google-play.json).
    if let Ok(facts) = &ctx.manifest {
        if facts.permissions.is_empty() {
            gates.push(Gate::new(
                "data_safety_consistency",
                "pass",
                "manifest declares zero permissions; a 'No data collected / shared' Data Safety posture is consistent with the build",
            ));
        } else {
            let declared = find_data_safety_declaration(project);
            match declared {
                Some(count) if count > 0 => gates.push(
                    Gate::new(
                        "data_safety_consistency",
                        "pass",
                        format!(
                            "manifest declares {} permissions and google-play.json declares {count} collected data types; declaration exists and is non-empty",
                            facts.permissions.len()
                        ),
                    )
                    .with_evidence(json!({"permissions": facts.permissions})),
                ),
                _ => gates.push(
                    Gate::new(
                        "data_safety_consistency",
                        "unverified",
                        format!(
                            "manifest declares {} permissions but no readable google-play.json data declaration was found to check consistency against",
                            facts.permissions.len()
                        ),
                    )
                    .with_evidence(json!({"permissions": facts.permissions})),
                ),
            }
        }
    }
    // Closed-test conditional: the rule fires only for personal
    // accounts created after 2023-11-13; the account type is
    // readable only in the Owner's Console and is not on record.
    gates.push(Gate::new(
        "closed_test_conditional",
        "conditional",
        "12-testers × 14-day closed test applies ONLY if the Play account is personal and created after 2023-11-13; account type/creation date are not on record (Console-only facts) — the condition cannot be resolved by measurement",
    ));
    gates.push(developer_verification_gate(project));
    gates
}

fn find_data_safety_declaration(project: &Project) -> Option<usize> {
    for dir in std::iter::once(project.listing_dir.clone())
        .flatten()
        .chain(project.extra_listing_dirs.iter().cloned())
    {
        let candidate = project.root.join(&dir).join("google-play.json");
        // The estate's google-play.json files carry a `//`
        // comment header (house style) and nest the declaration
        // under `dataSafety` — strip comment lines and accept
        // both nestings before judging the file unreadable.
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            let stripped: String = text
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            if let Ok(value) = serde_json::from_str::<Value>(&stripped) {
                // The declaration's nesting varies (the estate's
                // document keeps it under `privacy.dataSafety`);
                // search recursively for the dataCollected array.
                fn find_collected(value: &Value, depth: usize) -> Option<usize> {
                    if depth > 6 {
                        return None;
                    }
                    match value {
                        Value::Object(map) => {
                            if let Some(list) = map.get("dataCollected").and_then(Value::as_array) {
                                return Some(list.len());
                            }
                            map.values().find_map(|v| find_collected(v, depth + 1))
                        }
                        Value::Array(items) => {
                            items.iter().find_map(|v| find_collected(v, depth + 1))
                        }
                        _ => None,
                    }
                }
                if let Some(count) = find_collected(&value, 0) {
                    return Some(count);
                }
            }
        }
    }
    None
}

fn fdroid_battery(config: &Config, project: &Project, ctx: &Ctx) -> Vec<Gate> {
    let mut gates = artifact_gates(ctx);
    // Recipe presence + field rules (fdroid-publish Gate 3).
    let recipe_rel = project.fdroid_recipe.clone();
    match recipe_rel {
        None => gates.push(Gate::new(
            "fdroid_recipe",
            "unverified",
            "no F-Droid recipe path configured for this project",
        )),
        Some(rel) => match config.project_path(project, &rel) {
            Err(f) => gates.push(Gate::new("fdroid_recipe", "fail", f.message)),
            Ok(path) => match std::fs::read_to_string(&path) {
                Err(e) => gates.push(Gate::new(
                    "fdroid_recipe",
                    "fail",
                    format!("recipe {} unreadable: {e}", path.display()),
                )),
                Ok(text) => {
                    gates.push(recipe_gate(project, &text));
                }
            },
        },
    }
    // Fastlane tree completeness (fdroid-publish Gate 2).
    gates.push(fastlane_gate(config, project, ctx));
    // Listing images (shared-tree rule).
    gates.push(scan_listing_images(project, &listing_dirs(config, project)));
    // The project's own pre-flight check, where configured.
    if project.fdroid_check_command.is_empty() {
        gates.push(Gate::new(
            "fdroid_publish_check",
            "unverified",
            "no fdroid-publish-check command configured for this project",
        ));
    } else {
        // The configured argv may carry {root}, {package}, and
        // {version_code} tokens, substituted from the measured
        // manifest facts (fdroid-publish-check wants the exact
        // versionCode of the release being certified).
        let version_code = match &ctx.manifest {
            Ok(facts) => facts
                .version_code
                .map(|v| v.to_string())
                .unwrap_or_default(),
            Err(_) => String::new(),
        };
        let argv: Vec<String> = project
            .fdroid_check_command
            .iter()
            .map(|arg| {
                arg.replace("{root}", &project.root.to_string_lossy())
                    .replace("{package}", &project.package)
                    .replace("{version_code}", &version_code)
            })
            .collect();
        let outcome = exec::run(
            &argv,
            Some(&project.root),
            &std::collections::BTreeMap::new(),
            Duration::from_secs(120),
        );
        gates.push(Gate::new(
            "fdroid_publish_check",
            if outcome.ok() { "pass" } else { "fail" },
            format!(
                "fdroid-publish-check exit {:?}: {}",
                outcome.exit_code,
                outcome.brief().lines().last().unwrap_or("")
            ),
        ));
    }
    // FOSS eligibility (fdroid-publish Gate 1, light form).
    let license_ok = ["LICENSE", "LICENSE.md", "LICENSE.txt"]
        .iter()
        .filter_map(|name| std::fs::read_to_string(project.root.join(name)).ok())
        .any(|text| text.contains("Apache License") || text.contains("GNU GENERAL PUBLIC LICENSE"));
    gates.push(Gate::new(
        "foss_license",
        if license_ok { "pass" } else { "unverified" },
        if license_ok {
            "a FOSS license file (Apache/GPL family) is present at the repo root"
        } else {
            "no recognizable FOSS license file at the repo root"
        },
    ));
    gates.push(developer_verification_gate(project));
    gates
}

fn recipe_gate(project: &Project, text: &str) -> Gate {
    let mut problems: Vec<String> = Vec::new();
    if !text.lines().any(|l| l.trim_start().starts_with("License:")) {
        problems.push("no License: field".to_string());
    }
    let commits: Vec<&str> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("commit:"))
        .map(|s| s.trim().trim_matches(|c| c == '\'' || c == '"'))
        .collect();
    if commits.is_empty() {
        problems.push("no commit: pin in any Builds entry".to_string());
    }
    for commit in &commits {
        if !(commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit())) {
            problems.push(format!("commit pin {commit} is not a full 40-char SHA"));
        }
    }
    if !text.contains("UpdateCheckMode:") {
        problems.push("no UpdateCheckMode".to_string());
    }
    if let Some(pin) = &project.fdroid_pin
        && !commits.iter().any(|c| c == pin)
    {
        problems.push(format!(
                "channel-split pin {pin} not among the recipe's Builds commits — the recipe must not move past the ruled pin"
            ));
    }
    if problems.is_empty() {
        Gate::new(
            "fdroid_recipe",
            "pass",
            format!(
                "recipe carries License, {} full-SHA Builds pin(s), UpdateCheckMode",
                commits.len()
            ),
        )
        .with_evidence(json!({"commits": commits}))
    } else {
        Gate::new("fdroid_recipe", "fail", problems.join("; "))
            .with_evidence(json!({"commits": commits}))
    }
}

fn fastlane_gate(config: &Config, project: &Project, ctx: &Ctx) -> Gate {
    let Some(fastlane) = &project.fastlane_dir else {
        return Gate::new(
            "fastlane_tree",
            "unverified",
            "no fastlane metadata root configured (fastlane_dir)",
        );
    };
    let Ok(base) = config.project_path(project, fastlane) else {
        return Gate::new(
            "fastlane_tree",
            "fail",
            "listing tree path refused by the firewall",
        );
    };
    let lang = base.join("en-US");
    let mut problems: Vec<String> = Vec::new();
    let check_text = |rel: &str, max: usize, single_line: bool| -> Option<String> {
        match std::fs::read_to_string(lang.join(rel)) {
            Err(_) => Some(format!("missing {rel}")),
            Ok(text) => {
                let body = text.trim_end_matches('\n');
                if body.chars().count() > max {
                    Some(format!(
                        "{rel} is {} chars (max {max})",
                        body.chars().count()
                    ))
                } else if single_line && body.contains('\n') {
                    Some(format!("{rel} must be a single line"))
                } else {
                    None
                }
            }
        }
    };
    for problem in [
        check_text("title.txt", 50, false),
        check_text("short_description.txt", 80, true),
        check_text("full_description.txt", 4000, false),
    ]
    .into_iter()
    .flatten()
    {
        problems.push(problem);
    }
    match std::fs::read(lang.join("images/icon.png")) {
        Ok(bytes) => match artifact::png_facts(&bytes) {
            Some(facts) if facts.width == 512 && facts.height == 512 => {}
            Some(facts) => problems.push(format!(
                "icon.png is {}x{} (must be 512x512)",
                facts.width, facts.height
            )),
            None => problems.push("icon.png is not a PNG".to_string()),
        },
        Err(_) => problems.push("missing images/icon.png".to_string()),
    }
    if let Ok(facts) = &ctx.manifest
        && let Some(vc) = facts.version_code
    {
        let changelog = lang.join(format!("changelogs/{vc}.txt"));
        if !changelog.is_file() {
            problems.push(format!(
                "missing changelogs/{vc}.txt for the artifact's versionCode"
            ));
        }
    }
    if problems.is_empty() {
        Gate::new(
            "fastlane_tree",
            "pass",
            "fastlane tree complete for the artifact's versionCode",
        )
    } else {
        Gate::new("fastlane_tree", "fail", problems.join("; "))
    }
}

/// Run one channel's battery against one artifact of one
/// project. The artifact must live under the project root.
pub fn certify(
    config: &Config,
    project: &Project,
    channel: &str,
    artifact_path: &Path,
) -> Result<Certification, Failure> {
    let canon = config.check_artifact_path(project, artifact_path)?;
    let census = artifact::census(&canon)?;
    let manifest = android::manifest_facts(config, &canon);
    let signature = android::signature_facts(config, &canon);
    let ctx = Ctx {
        census,
        manifest,
        signature,
    };
    let gates = match channel {
        "google-play" => play_battery(config, project, &ctx),
        "fdroid" => fdroid_battery(config, project, &ctx),
        other => {
            return Err(Failure::params(format!(
                "unknown channel: {other} (expected google-play or fdroid)"
            )));
        }
    };
    Ok(Certification {
        project: project.id.clone(),
        channel: channel.to_string(),
        issued: exec::now_rfc3339(),
        artifact: json!({
            "path": canon,
            "sha256": ctx.census.sha256,
            "size": ctx.census.size,
            "signature_state": ctx.signature.state,
        }),
        gates,
    })
}
