//! The pipeline state machine: build → sign → verify →
//! install → release as staged steps, with the approval rules
//! encoded as pure transitions.
//!
//! Approval rule (the consequential-step contract):
//! - **Plan mode:** every step is staged (exact tool + args
//!   shown) and waits for a human approval before it runs.
//! - **Auto mode:** build and verify run freely; a consequential
//!   step (sign / install / release) stops for a human approval
//!   *unless* slipway's own certification gates are all green —
//!   only the member's own gates can authorize, never the TUI.
//!
//! Progress honesty: a step carries no percentage. Its progress
//! text is its state plus, while running, the elapsed time and
//! the count of log lines the server itself reported.

use std::time::{Duration, Instant};

/// How runs are authorized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Stage every call; a human approves each step.
    Plan,
    /// Run free steps directly; consequential steps still stop
    /// unless the member's gates authorize them.
    Auto,
}

impl Mode {
    /// The other mode (for the toggle key).
    pub fn toggled(self) -> Mode {
        match self {
            Mode::Plan => Mode::Auto,
            Mode::Auto => Mode::Plan,
        }
    }
    /// Short label for headers.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Plan => "plan",
            Mode::Auto => "auto",
        }
    }
}

/// One pipeline stage, in run order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// `slipway_build` — free.
    Build,
    /// `slipway_sign` — consequential.
    Sign,
    /// `slipway_verify` — free.
    Verify,
    /// `slipway_install` — consequential.
    Install,
    /// `slipway_release` — consequential.
    Release,
}

impl StepKind {
    /// All steps in pipeline order.
    pub const ORDER: [StepKind; 5] = [
        StepKind::Build,
        StepKind::Sign,
        StepKind::Verify,
        StepKind::Install,
        StepKind::Release,
    ];

    /// The MCP tool that performs this step.
    pub fn tool(self) -> &'static str {
        match self {
            StepKind::Build => "slipway_build",
            StepKind::Sign => "slipway_sign",
            StepKind::Verify => "slipway_verify",
            StepKind::Install => "slipway_install",
            StepKind::Release => "slipway_release",
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            StepKind::Build => "build",
            StepKind::Sign => "sign",
            StepKind::Verify => "verify",
            StepKind::Install => "install",
            StepKind::Release => "release",
        }
    }

    /// Whether this step changes the world (signs, installs, or
    /// releases) and therefore stops for approval in auto mode.
    pub fn is_consequential(self) -> bool {
        matches!(self, StepKind::Sign | StepKind::Install | StepKind::Release)
    }
}

/// The state of one step. Invalid combinations are
/// unrepresentable: a step is in exactly one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepState {
    /// Not yet reached.
    Pending,
    /// Staged, waiting for a human approval.
    AwaitingApproval,
    /// Dispatched; the server has not finished it.
    Running,
    /// The server reported success.
    Done,
    /// The server reported failure, with its message.
    Failed(String),
    /// The server died (or vanished) around this step: the
    /// outcome is not known and must never render as success.
    Unknown(String),
    /// Never ran because the pipeline stopped earlier or the
    /// staged step was rejected.
    Skipped,
}

/// One step's runtime record.
#[derive(Debug, Clone)]
pub struct Step {
    /// Which stage this is.
    pub kind: StepKind,
    /// Its current state.
    pub state: StepState,
    /// When dispatch happened (for elapsed display).
    pub started: Option<Instant>,
    /// How long it ran, once finished.
    pub elapsed: Option<Duration>,
    /// Log lines the *server* reported for this step, verbatim.
    pub server_log_lines: usize,
}

impl Step {
    fn new(kind: StepKind) -> Step {
        Step {
            kind,
            state: StepState::Pending,
            started: None,
            elapsed: None,
            server_log_lines: 0,
        }
    }

    /// Honest progress text: state + elapsed while running.
    /// There is intentionally no percentage anywhere: the
    /// server either reported log lines or it did not, and a
    /// silent step shows only that time is passing.
    pub fn progress_text(&self, now: Instant) -> String {
        match &self.state {
            StepState::Pending => "pending".to_string(),
            StepState::AwaitingApproval => "staged — awaiting approval".to_string(),
            StepState::Running => {
                let secs = self
                    .started
                    .map(|s| now.duration_since(s).as_secs())
                    .unwrap_or(0);
                if self.server_log_lines == 0 {
                    format!(
                        "running — elapsed {secs}s, no progress events \
                         reported by server"
                    )
                } else {
                    format!(
                        "running — elapsed {secs}s, {} log lines from server",
                        self.server_log_lines
                    )
                }
            }
            StepState::Done => match self.elapsed {
                Some(d) => format!("done in {}s", d.as_secs()),
                None => "done".to_string(),
            },
            StepState::Failed(m) => format!("FAILED: {m}"),
            StepState::Unknown(m) => format!("UNKNOWN: {m}"),
            StepState::Skipped => "skipped".to_string(),
        }
    }
}

/// Whole-pipeline state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineState {
    /// No run in flight.
    Idle,
    /// A step is dispatched or steps remain.
    Running,
    /// Stopped at a staged step, waiting for approval.
    AwaitingApproval,
    /// Every step reported done.
    Completed,
    /// A step failed, or the server died.
    Failed,
    /// A human rejected a staged step.
    Aborted,
}

/// The pipeline: ordered steps plus the authorization context.
#[derive(Debug, Clone)]
pub struct Pipeline {
    /// Steps in run order.
    pub steps: Vec<Step>,
    /// Whole-pipeline state.
    pub state: PipelineState,
    /// Whether slipway's own certification gates are all green
    /// (the only thing that lets auto mode pass a consequential
    /// step without a human).
    pub gates_authorized: bool,
}

impl Pipeline {
    /// A fresh, idle pipeline.
    pub fn new() -> Pipeline {
        Pipeline {
            steps: StepKind::ORDER.iter().map(|k| Step::new(*k)).collect(),
            state: PipelineState::Idle,
            gates_authorized: false,
        }
    }

    /// Reset every step for a new run (gate authorization is
    /// kept: it describes the member, not the run).
    pub fn reset(&mut self) {
        for step in &mut self.steps {
            *step = Step::new(step.kind);
        }
        self.state = PipelineState::Idle;
    }

    /// The pure approval rule: does `kind` stop for a human
    /// before running, under `mode` and the current gate
    /// authorization?
    pub fn must_stop_before(kind: StepKind, mode: Mode, gates_authorized: bool) -> bool {
        match mode {
            Mode::Plan => true,
            Mode::Auto => kind.is_consequential() && !gates_authorized,
        }
    }

    /// Index of the first step that has not finished, if any.
    pub fn current_index(&self) -> Option<usize> {
        self.steps.iter().position(|s| {
            matches!(
                s.state,
                StepState::Pending | StepState::AwaitingApproval | StepState::Running
            )
        })
    }

    /// Whether every step reported done.
    pub fn all_done(&self) -> bool {
        self.steps.iter().all(|s| s.state == StepState::Done)
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Pipeline::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_mode_stops_before_every_step() {
        for kind in StepKind::ORDER {
            assert!(Pipeline::must_stop_before(kind, Mode::Plan, false));
            assert!(Pipeline::must_stop_before(kind, Mode::Plan, true));
        }
    }

    #[test]
    fn auto_mode_runs_free_steps_and_stops_at_sign() {
        assert!(!Pipeline::must_stop_before(
            StepKind::Build,
            Mode::Auto,
            false
        ));
        assert!(!Pipeline::must_stop_before(
            StepKind::Verify,
            Mode::Auto,
            false
        ));
        // The core safety rule: auto stops before sign, install,
        // and release when the member's gates have not authorized.
        assert!(Pipeline::must_stop_before(
            StepKind::Sign,
            Mode::Auto,
            false
        ));
        assert!(Pipeline::must_stop_before(
            StepKind::Install,
            Mode::Auto,
            false
        ));
        assert!(Pipeline::must_stop_before(
            StepKind::Release,
            Mode::Auto,
            false
        ));
    }

    #[test]
    fn auto_mode_passes_consequential_only_when_gates_authorize() {
        assert!(!Pipeline::must_stop_before(
            StepKind::Sign,
            Mode::Auto,
            true
        ));
        assert!(!Pipeline::must_stop_before(
            StepKind::Release,
            Mode::Auto,
            true
        ));
    }

    #[test]
    fn silent_running_step_shows_elapsed_and_no_percentage() {
        let mut step = Step::new(StepKind::Build);
        step.state = StepState::Running;
        let start = Instant::now();
        step.started = Some(start);
        let text = step.progress_text(start + Duration::from_secs(42));
        assert!(text.contains("elapsed 42s"), "{text}");
        assert!(text.contains("no progress events"), "{text}");
        assert!(!text.contains('%'), "fabricated percentage in: {text}");
    }

    #[test]
    fn unknown_never_renders_as_done() {
        let mut step = Step::new(StepKind::Sign);
        step.state = StepState::Unknown("server died".into());
        let text = step.progress_text(Instant::now());
        assert!(text.starts_with("UNKNOWN"), "{text}");
        assert!(!text.contains("done"), "{text}");
    }
}
