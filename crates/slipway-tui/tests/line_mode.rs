//! Headless smoke: line mode answers commands from a pipe
//! against the fake facade, and the plan-mode run stages its
//! first step without any workflow call leaking through.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_line_mode(input: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_slipway-tui"))
        .args(["--fake", "--line"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn slipway-tui");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write commands");
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn line_mode_renders_overview_and_fleet() {
    let out = run_line_mode("overview\nfleet\nquit\n");
    assert!(out.contains("slipway 0.1.0"), "{out}");
    assert!(out.contains("tools: 15"), "{out}");
    assert!(out.contains("slipway-pipeline-to-bloch-lesson"), "{out}");
    assert!(out.contains("bloch.lesson_candidate"), "{out}");
}

#[test]
fn line_mode_plan_run_stages_first_step() {
    let out = run_line_mode("workflows\nrun\nworkflows\nquit\n");
    assert!(out.contains("staged call: slipway_build"), "{out}");
    assert!(out.contains("awaiting approval"), "{out}");
}
