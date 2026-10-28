//! Bounded subprocess execution: every external tool slipway
//! runs (Gradle, adb, aapt2, apksigner, readelf-class probes)
//! goes through this runner — a named timeout, capped captured
//! output, and an honest record of the argv that ran. Nothing
//! here ever sees a secret value: signing passwords move only
//! through the environment of the child (see `signing`).

use std::collections::BTreeMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Captured-output cap per stream (bytes).
pub const OUTPUT_BYTES_MAX: usize = 1024 * 1024;
/// Default per-command deadline.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Build deadline: native Android builds are the long pole.
pub const BUILD_TIMEOUT_SECS: u64 = 2400;

/// The outcome of one bounded run.
pub struct Outcome {
    pub argv: Vec<String>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub elapsed_ms: u128,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        !self.timed_out && self.exit_code == Some(0)
    }

    /// Combined trimmed output, for evidence strings.
    pub fn brief(&self) -> String {
        let mut text = self.stdout.trim().to_string();
        if !self.stderr.trim().is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(self.stderr.trim());
        }
        text
    }
}

/// Run one command with a deadline and capped output. Spawn
/// failure is an outcome with `exit_code: None`, not a panic.
pub fn run(
    argv: &[String],
    cwd: Option<&std::path::Path>,
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Outcome {
    let started = Instant::now();
    let argv_owned: Vec<String> = argv.to_vec();
    if argv.is_empty() {
        return Outcome {
            argv: argv_owned,
            exit_code: None,
            timed_out: false,
            stdout: String::new(),
            stderr: "empty argv".to_string(),
            elapsed_ms: 0,
        };
    }
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(env.iter());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return Outcome {
                argv: argv_owned,
                exit_code: None,
                timed_out: false,
                stdout: String::new(),
                stderr: format!("spawn failed: {e}"),
                elapsed_ms: started.elapsed().as_millis(),
            };
        }
    };
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stdout_pipe.take() {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe.take() {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if started.elapsed() > timeout {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break None,
        }
    };
    let stdout_raw = out_thread.join().unwrap_or_default();
    let stderr_raw = err_thread.join().unwrap_or_default();
    Outcome {
        argv: argv_owned,
        exit_code: status.and_then(|s| s.code()),
        timed_out,
        stdout: cap(&stdout_raw),
        stderr: cap(&stderr_raw),
        elapsed_ms: started.elapsed().as_millis(),
    }
}

fn cap(bytes: &[u8]) -> String {
    let slice = if bytes.len() > OUTPUT_BYTES_MAX {
        &bytes[..OUTPUT_BYTES_MAX]
    } else {
        bytes
    };
    String::from_utf8_lossy(slice).into_owned()
}

/// SHA-256 of a file, streamed.
pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use sha2::Digest;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// SHA-256 of bytes.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

/// RFC 3339 timestamp for now (receipts, records).
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Unix seconds for now (manifest freshness math).
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}
