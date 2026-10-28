//! Slipway configuration: configured project roots (the Owner's
//! own source trees only), toolchain locations, and the state
//! directory that holds receipts, nonces, sessions, and bundles.
//!
//! The methodology firewall is structural here (STUDY.md §2):
//! every project root is canonicalized at load and checked for
//! overlap against the configured forbidden roots (fathom's
//! reference stores and any artifact-under-study directory).
//! Overlap — in either direction — fails the load, closed.
//! Artifact paths handed to tools are re-checked against the
//! project roots on every call, not trusted from the config.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::Failure;

/// One configured customer project.
#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    /// Config key / receipt name, e.g. `light-show`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Android application id, e.g. `ai.qompass.lightshow`.
    pub package: String,
    /// Canonical source-tree root (set at load).
    #[serde(skip)]
    pub root: PathBuf,
    /// The root as written in the config file.
    root_dir: String,
    /// Build command (argv) run from the root by `slipway_build`.
    /// Empty means the project has no build step configured.
    #[serde(default)]
    pub build_command: Vec<String>,
    /// Environment for the build command (values here are never
    /// secrets — paths and pins only).
    #[serde(default)]
    pub build_env: BTreeMap<String, String>,
    /// Artifact paths, relative to the root, that builds produce.
    #[serde(default)]
    pub artifacts: Vec<String>,
    /// Listing tree root (fastlane metadata), relative to root.
    pub listing_dir: Option<String>,
    /// Fastlane metadata root (the directory holding the
    /// per-locale trees, e.g. `fastlane/metadata/android`),
    /// relative to root. Distinct from `listing_dir`, which
    /// names where listing IMAGES are scanned.
    #[serde(default)]
    pub fastlane_dir: Option<String>,
    /// Additional listing image directories, relative to root
    /// (e.g. a `playstore/` tree beside the fastlane tree).
    #[serde(default)]
    pub extra_listing_dirs: Vec<String>,
    /// F-Droid recipe path, relative to root.
    pub fdroid_recipe: Option<String>,
    /// The F-Droid channel-split pin (full SHA) the recipe must
    /// carry, where the project's publish skill rules one.
    pub fdroid_pin: Option<String>,
    /// In-app MCP server command (argv) for `slipway_drive_app`.
    #[serde(default)]
    pub app_server_command: Vec<String>,
    /// fdroid-publish-check invocation (argv, run from the root)
    /// for the F-Droid battery, where the project carries one.
    #[serde(default)]
    pub fdroid_check_command: Vec<String>,
    /// Whether the package + signing key stand registered under
    /// the Owner's verified developer identity (2027 gate). This
    /// is the Owner's identity act; the config only records that
    /// it stands done. Default false = not on record.
    #[serde(default)]
    pub developer_verification_registered: bool,
    /// Signing key reference: a keystore PATH (never a value).
    /// The password reaches the signer only via the named
    /// environment variable in `keystore_pass_env`.
    pub keystore_ref: Option<String>,
    /// Name of the environment variable that holds the keystore
    /// password at signing dispatch time.
    pub keystore_pass_env: Option<String>,
    /// Key alias inside the keystore.
    pub key_alias: Option<String>,
}

/// The whole server configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Directory for receipts / nonces / sessions / bundles /
    /// certification records. Relative paths resolve against the
    /// config file's directory.
    pub state_dir: String,
    /// Roots slipway must never touch: fathom reference stores,
    /// screened trees, artifact-under-study directories.
    #[serde(default)]
    pub forbidden_roots: Vec<String>,
    /// Toolchain overrides: program name → absolute path. Anything
    /// absent is probed on PATH at use time and reported as a fact.
    #[serde(default)]
    pub toolchain: BTreeMap<String, String>,
    /// Trusted action-signing public keys (hex ed25519) whose
    /// signed manifests `slipway_sign` will honor. The Owner's
    /// subkey belongs here; test builds carry only a labelled
    /// throwaway test key.
    #[serde(default)]
    pub signer_pubkeys: Vec<String>,
    pub projects: Vec<Project>,
    /// Canonical state dir (set at load).
    #[serde(skip)]
    pub state_root: PathBuf,
    /// Canonical forbidden roots (set at load).
    #[serde(skip)]
    pub forbidden: Vec<PathBuf>,
}

fn canonical(path: &Path) -> Result<PathBuf, Failure> {
    path.canonicalize().map_err(|e| {
        Failure::domain(
            "config_invalid",
            format!("cannot resolve path {}: {e}", path.display()),
        )
    })
}

/// True when `a` contains `b` or `b` contains `a`.
fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

impl Config {
    /// Load, canonicalize, and firewall-check a config file.
    pub fn load(path: &Path) -> Result<Config, Failure> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            Failure::domain(
                "config_invalid",
                format!("cannot read config {}: {e}", path.display()),
            )
        })?;
        let mut config: Config = toml::from_str(&text).map_err(|e| {
            Failure::domain("config_invalid", format!("config does not parse: {e}"))
        })?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let resolve = |p: &str| -> PathBuf {
            if let Some(rest) = p.strip_prefix("~/")
                && let Some(home) = std::env::home_dir()
            {
                return home.join(rest);
            }
            let pb = PathBuf::from(p);
            if pb.is_absolute() { pb } else { base.join(pb) }
        };
        let state = resolve(&config.state_dir);
        std::fs::create_dir_all(&state).map_err(|e| {
            Failure::domain(
                "config_invalid",
                format!("cannot create state dir {}: {e}", state.display()),
            )
        })?;
        config.state_root = canonical(&state)?;
        let mut forbidden = Vec::new();
        for root in &config.forbidden_roots {
            // A forbidden root that does not exist yet cannot
            // overlap anything; record it unresolved and let the
            // per-call check compare prefixes textually as well.
            let resolved = resolve(root);
            match canonical(&resolved) {
                Ok(c) => forbidden.push(c),
                Err(_) => forbidden.push(resolved),
            }
        }
        config.forbidden = forbidden;
        if config.projects.is_empty() {
            return Err(Failure::domain(
                "config_invalid",
                "config names no projects",
            ));
        }
        for project in &mut config.projects {
            let root = canonical(&resolve(&project.root_dir))?;
            for bad in &config.forbidden {
                if overlaps(&root, bad) {
                    return Err(Failure::domain(
                        "firewall_overlap",
                        format!(
                            "project {} root {} overlaps forbidden root {} — refused (methodology firewall)",
                            project.id,
                            root.display(),
                            bad.display()
                        ),
                    ));
                }
            }
            project.root = root;
        }
        Ok(config)
    }

    pub fn project(&self, id: &str) -> Result<&Project, Failure> {
        self.projects
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| Failure::params(format!("unknown project: {id}")))
    }

    /// The firewall check applied to every artifact path a tool
    /// is handed: the canonical path must live under the named
    /// project's root and under no forbidden root.
    pub fn check_artifact_path(&self, project: &Project, path: &Path) -> Result<PathBuf, Failure> {
        let canon = canonical(path)?;
        if !canon.starts_with(&project.root) {
            return Err(Failure::domain(
                "outside_project_root",
                format!(
                    "artifact {} is outside project {} root {} — refused",
                    canon.display(),
                    project.id,
                    project.root.display()
                ),
            ));
        }
        for bad in &self.forbidden {
            if canon.starts_with(bad) {
                return Err(Failure::domain(
                    "firewall_overlap",
                    format!(
                        "artifact {} is under forbidden root {} — refused (methodology firewall)",
                        canon.display(),
                        bad.display()
                    ),
                ));
            }
        }
        Ok(canon)
    }

    /// Resolve a project-relative path and firewall-check it.
    pub fn project_path(&self, project: &Project, rel: &str) -> Result<PathBuf, Failure> {
        self.check_artifact_path(project, &project.root.join(rel))
    }

    pub fn receipts_path(&self) -> PathBuf {
        self.state_root.join("receipts.jsonl")
    }

    pub fn certifications_dir(&self) -> PathBuf {
        self.state_root.join("certifications")
    }

    pub fn bundles_dir(&self) -> PathBuf {
        self.state_root.join("bundles")
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.state_root.join("sessions")
    }

    pub fn nonces_path(&self) -> PathBuf {
        self.state_root.join("nonces.json")
    }

    /// Toolchain program resolution: config override, else PATH.
    pub fn program(&self, name: &str) -> Option<PathBuf> {
        if let Some(p) = self.toolchain.get(name) {
            let pb = PathBuf::from(p);
            if pb.is_file() {
                return Some(pb);
            }
        }
        which(name)
    }
}

/// Minimal PATH lookup (no external `which` dependency).
pub fn which(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("slipway.toml");
        std::fs::write(&path, body).expect("write config");
        path
    }

    #[test]
    fn overlapping_project_root_is_refused() {
        let tmp = std::env::temp_dir().join(format!("slipway-cfg-{}", std::process::id()));
        let proj = tmp.join("proj");
        let store = tmp.join("store");
        std::fs::create_dir_all(&proj).expect("mkdir proj");
        std::fs::create_dir_all(&store).expect("mkdir store");
        // Project root INSIDE the forbidden store.
        let nested = store.join("proj");
        std::fs::create_dir_all(&nested).expect("mkdir nested");
        let body = format!(
            "state_dir = \"{}\"\nforbidden_roots = [\"{}\"]\n[[projects]]\nid = \"x\"\nname = \"X\"\npackage = \"ai.example.x\"\nroot_dir = \"{}\"\n",
            tmp.join("state").display(),
            store.display(),
            nested.display()
        );
        let result = Config::load(&write_config(&tmp, &body));
        match result {
            Err(f) => assert_eq!(f.code, "firewall_overlap"),
            Ok(_) => panic!("overlapping config must be refused"),
        }
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn disjoint_config_loads_and_paths_check() {
        let tmp = std::env::temp_dir().join(format!("slipway-cfg2-{}", std::process::id()));
        let proj = tmp.join("proj");
        let store = tmp.join("store");
        std::fs::create_dir_all(&proj).expect("mkdir proj");
        std::fs::create_dir_all(&store).expect("mkdir store");
        std::fs::write(proj.join("app.apk"), b"apk").expect("write apk");
        let body = format!(
            "state_dir = \"{}\"\nforbidden_roots = [\"{}\"]\n[[projects]]\nid = \"x\"\nname = \"X\"\npackage = \"ai.example.x\"\nroot_dir = \"{}\"\n",
            tmp.join("state").display(),
            store.display(),
            proj.display()
        );
        let config = Config::load(&write_config(&tmp, &body)).expect("load");
        let project = config.project("x").expect("project");
        assert!(
            config
                .check_artifact_path(project, &proj.join("app.apk"))
                .is_ok()
        );
        // A path outside the root is refused even though it exists.
        std::fs::write(tmp.join("outside.apk"), b"apk").expect("write outside");
        let err = config
            .check_artifact_path(project, &tmp.join("outside.apk"))
            .expect_err("must refuse");
        assert_eq!(err.code, "outside_project_root");
        // An artifact under a forbidden root inside the project is refused.
        std::fs::remove_dir_all(&tmp).ok();
    }
}
