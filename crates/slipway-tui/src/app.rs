//! The app controller: screen state, the facade worker thread,
//! and every state transition the screens render.
//!
//! All facade access runs on one worker thread so a long build
//! never blocks rendering: the UI thread sends commands and
//! drains typed events on each tick. Step state therefore
//! reflects the server's real reporting — started, the log
//! lines it emitted, done / failed — and nothing else. A step
//! the server says nothing about simply ages (elapsed time in
//! the panel), because there is no progress value to show.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Instant;

use serde_json::{Value, json};

use crate::facade::{FacadeError, SlipwayFacade, step_args};
use crate::pipeline::{Mode, Pipeline, PipelineState, StepState};
use crate::screens::Screen;
use crate::triggers::TriggerEngine;

/// Log lines retained in the Log screen buffer.
pub const LOG_LINES_MAX: usize = 500;
/// Upper bound on pump iterations while waiting for the worker.
pub const PUMP_ATTEMPTS_MAX: usize = 500;

/// One measured certification gate, as the server reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRow {
    /// Gate name.
    pub name: String,
    /// Measured result string from the server (`pass`/`fail`/…).
    pub result: String,
    /// The server's detail line.
    pub detail: String,
}

/// A certification-gate checklist parsed from a
/// `slipway_certify` result.
#[derive(Debug, Clone, PartialEq)]
pub struct GateChecklist {
    /// Project the certification covers.
    pub project: String,
    /// Channel the certification covers.
    pub channel: String,
    /// The server's verdict string, if it gave one.
    pub verdict: String,
    /// Every gate, in the server's order.
    pub gates: Vec<GateRow>,
}

impl GateChecklist {
    /// Parse a `slipway_certify` tool result. Unknown shapes are
    /// an error, not an empty checklist — an empty checklist
    /// must never read as "all green".
    pub fn parse(value: &Value) -> Result<GateChecklist, FacadeError> {
        let raw = value["gates"]
            .as_array()
            .ok_or_else(|| FacadeError::Message("certify result has no gates array".into()))?;
        let gates = raw
            .iter()
            .map(|g| GateRow {
                name: g["name"]
                    .as_str()
                    .or_else(|| g["gate"].as_str())
                    .unwrap_or("unnamed gate")
                    .to_string(),
                result: g["result"].as_str().unwrap_or("unknown").to_string(),
                detail: g["detail"].as_str().unwrap_or("").to_string(),
            })
            .collect();
        Ok(GateChecklist {
            project: value["project"].as_str().unwrap_or("").to_string(),
            channel: value["channel"].as_str().unwrap_or("").to_string(),
            verdict: value["verdict"].as_str().unwrap_or("").to_string(),
            gates,
        })
    }

    /// True only when at least one gate exists and every gate
    /// the server measured passed.
    pub fn all_green(&self) -> bool {
        !self.gates.is_empty() && self.gates.iter().all(|g| g.result == "pass")
    }

    /// `(passed, total)` for summary lines.
    pub fn passed(&self) -> (usize, usize) {
        (
            self.gates.iter().filter(|g| g.result == "pass").count(),
            self.gates.len(),
        )
    }
}

enum Command {
    Status,
    Tools,
    Gates {
        project: String,
        channel: String,
        artifact: String,
    },
    Step {
        index: usize,
        tool: String,
        args: Value,
    },
    Shutdown,
}

enum Event {
    Status(Result<Value, FacadeError>),
    Tools(Result<Vec<Value>, FacadeError>),
    Gates(Result<GateChecklist, FacadeError>),
    StepStarted {
        index: usize,
    },
    StepLog {
        index: usize,
        line: String,
    },
    StepFinished {
        index: usize,
        result: Result<Value, FacadeError>,
    },
}

fn run_worker(
    mut facade: Box<dyn SlipwayFacade>,
    commands: Receiver<Command>,
    events: Sender<Event>,
) {
    while let Ok(command) = commands.recv() {
        match command {
            Command::Shutdown => break,
            Command::Status => {
                let _ = events.send(Event::Status(facade.status()));
            }
            Command::Tools => {
                let _ = events.send(Event::Tools(facade.tools()));
            }
            Command::Gates {
                project,
                channel,
                artifact,
            } => {
                let args = json!({"project": project, "channel": channel,
                                  "artifact": artifact});
                let result = facade
                    .call("slipway_certify", args)
                    .and_then(|v| GateChecklist::parse(&v));
                let _ = events.send(Event::Gates(result));
            }
            Command::Step { index, tool, args } => {
                let _ = events.send(Event::StepStarted { index });
                let result = facade.call(&tool, args);
                if let Ok(value) = &result
                    && let Some(lines) = value["log"].as_array()
                {
                    for line in lines {
                        if let Some(text) = line.as_str() {
                            let _ = events.send(Event::StepLog {
                                index,
                                line: text.to_string(),
                            });
                        }
                    }
                }
                let _ = events.send(Event::StepFinished { index, result });
            }
        }
    }
}

/// The whole TUI state. Screens render from this; tests drive
/// it through the same methods the keys use.
pub struct App {
    /// Current screen.
    pub screen: Screen,
    /// Plan / auto authorization mode.
    pub mode: Mode,
    /// The pipeline state machine.
    pub pipeline: Pipeline,
    /// Latest status report, once loaded.
    pub status: Option<Value>,
    /// Tool count, once loaded.
    pub tool_count: Option<usize>,
    /// Whether the server is believed alive.
    pub server_alive: bool,
    /// Latest certification checklist, once loaded.
    pub gates: Option<GateChecklist>,
    /// Target project id for workflow runs.
    pub project: String,
    /// Target channel for release / certify.
    pub channel: String,
    /// Target artifact for sign / verify / install / certify.
    pub artifact: String,
    /// Bounded event log (the Log screen).
    pub log: VecDeque<String>,
    /// The trigger engine (Fleet panel state).
    pub triggers: TriggerEngine,
    target_from_status: bool,
    commands: Sender<Command>,
    events: Receiver<Event>,
    worker: Option<JoinHandle<()>>,
}

impl App {
    /// Build the app over a facade and declared trigger rules,
    /// spawn the worker, and request status + tools.
    pub fn new(facade: Box<dyn SlipwayFacade>, triggers: TriggerEngine) -> App {
        let (cmd_tx, cmd_rx) = channel();
        let (evt_tx, evt_rx) = channel();
        let worker = std::thread::spawn(move || run_worker(facade, cmd_rx, evt_tx));
        let mut app = App {
            screen: Screen::Overview,
            mode: Mode::Plan,
            pipeline: Pipeline::new(),
            status: None,
            tool_count: None,
            server_alive: true,
            gates: None,
            project: "demo".to_string(),
            channel: "internal".to_string(),
            artifact: "app.aab".to_string(),
            log: VecDeque::new(),
            triggers,
            target_from_status: false,
            commands: cmd_tx,
            events: evt_rx,
            worker: Some(worker),
        };
        app.request_status();
        app.request_tools();
        app
    }

    /// Append one line to the bounded log.
    pub fn push_log(&mut self, line: String) {
        if self.log.len() >= LOG_LINES_MAX {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    /// Ask the worker for a fresh status report.
    pub fn request_status(&mut self) {
        let _ = self.commands.send(Command::Status);
    }

    /// Ask the worker for the tool list.
    pub fn request_tools(&mut self) {
        let _ = self.commands.send(Command::Tools);
    }

    /// Ask the worker to run the certification battery for the
    /// current target and load the checklist.
    pub fn request_gates(&mut self) {
        let _ = self.commands.send(Command::Gates {
            project: self.project.clone(),
            channel: self.channel.clone(),
            artifact: self.artifact.clone(),
        });
    }

    /// Start a pipeline run from the first step.
    pub fn run_pipeline(&mut self) {
        self.pipeline.reset();
        self.pipeline.state = PipelineState::Running;
        self.push_log(format!(
            "pipeline started: project {} channel {} mode {}",
            self.project,
            self.channel,
            self.mode.label()
        ));
        self.advance();
    }

    /// Approve the currently staged step (plan mode, or an auto
    /// mode stop at a consequential step).
    pub fn approve(&mut self) {
        if let Some(index) = self.awaiting_index() {
            self.push_log(format!(
                "approved: {}",
                self.pipeline.steps[index].kind.tool()
            ));
            self.dispatch_step(index);
        }
    }

    /// Reject the currently staged step: the run aborts and no
    /// facade call for that step is ever made.
    pub fn reject(&mut self) {
        if let Some(index) = self.awaiting_index() {
            self.pipeline.steps[index].state = StepState::Skipped;
            for step in &mut self.pipeline.steps {
                if step.state == StepState::Pending {
                    step.state = StepState::Skipped;
                }
            }
            self.pipeline.state = PipelineState::Aborted;
            self.push_log("staged step rejected — pipeline aborted".to_string());
        }
    }

    /// Toggle plan/auto (refused mid-run: authorization terms
    /// must not change under a live pipeline).
    pub fn toggle_mode(&mut self) {
        if self.pipeline.state == PipelineState::Idle
            || self.pipeline.state == PipelineState::Completed
            || self.pipeline.state == PipelineState::Failed
            || self.pipeline.state == PipelineState::Aborted
        {
            self.mode = self.mode.toggled();
        }
    }

    fn awaiting_index(&self) -> Option<usize> {
        self.pipeline
            .steps
            .iter()
            .position(|s| s.state == StepState::AwaitingApproval)
    }

    fn dispatch_step(&mut self, index: usize) {
        let kind = self.pipeline.steps[index].kind;
        let args = step_args(kind.tool(), &self.project, &self.channel, &self.artifact);
        self.pipeline.steps[index].state = StepState::Running;
        self.pipeline.steps[index].started = Some(Instant::now());
        self.pipeline.state = PipelineState::Running;
        let _ = self.commands.send(Command::Step {
            index,
            tool: kind.tool().to_string(),
            args,
        });
    }

    /// Move the pipeline forward: dispatch the next free step,
    /// stage the next approval-stopped step, or complete.
    fn advance(&mut self) {
        let Some(index) = self.pipeline.current_index() else {
            self.complete_pipeline();
            return;
        };
        let step = &self.pipeline.steps[index];
        if step.state != StepState::Pending {
            return; // running or already staged
        }
        let kind = step.kind;
        if Pipeline::must_stop_before(kind, self.mode, self.pipeline.gates_authorized) {
            let args = step_args(kind.tool(), &self.project, &self.channel, &self.artifact);
            self.pipeline.steps[index].state = StepState::AwaitingApproval;
            self.pipeline.state = PipelineState::AwaitingApproval;
            self.push_log(format!(
                "staged {} {} — awaiting approval",
                kind.tool(),
                args
            ));
            return;
        }
        self.dispatch_step(index);
    }

    fn complete_pipeline(&mut self) {
        if !self.pipeline.all_done() {
            return;
        }
        self.pipeline.state = PipelineState::Completed;
        self.push_log("pipeline completed: all steps reported done".to_string());
        let green = self.gates.as_ref().is_some_and(GateChecklist::all_green);
        let payload = format!(
            "pipeline completed for project {} (certification gates green: {green})",
            self.project
        );
        let fired = self
            .triggers
            .evaluate("pipeline.completed", green, &payload);
        for firing in fired {
            self.push_log(format!(
                "trigger fired: {} -> {}.{} (recorded; live dispatch \
                 is not wired in rung 1)",
                firing.rule_id, firing.target_mcp, firing.target_tool
            ));
        }
    }

    fn mark_server_died(&mut self, index: usize, detail: &str) {
        self.server_alive = false;
        if let Some(step) = self.pipeline.steps.get_mut(index)
            && step.state == StepState::Running
        {
            step.state = StepState::Unknown(detail.to_string());
        }
        for step in &mut self.pipeline.steps {
            if matches!(step.state, StepState::Pending | StepState::AwaitingApproval) {
                step.state = StepState::Unknown("server unreachable".to_string());
            }
        }
        self.pipeline.state = PipelineState::Failed;
        self.push_log(format!("server died mid-pipeline: {detail}"));
    }

    /// Drain every pending worker event. Returns how many were
    /// applied.
    pub fn tick(&mut self) -> usize {
        let mut applied = 0;
        while let Ok(event) = self.events.try_recv() {
            applied += 1;
            self.apply_event(event);
        }
        applied
    }

    /// Tick until the pipeline settles (no step running and no
    /// run in flight) or the attempt bound is hit. Used by line
    /// mode and tests; the full-screen loop ticks on its own
    /// cadence instead.
    pub fn pump_until_settled(&mut self) {
        for _ in 0..PUMP_ATTEMPTS_MAX {
            self.tick();
            let running = self
                .pipeline
                .steps
                .iter()
                .any(|s| s.state == StepState::Running);
            if !running && self.pipeline.state != PipelineState::Running {
                // One extra tick so trailing replies (status,
                // gates) land before the caller reads state.
                std::thread::sleep(std::time::Duration::from_millis(2));
                self.tick();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn apply_event(&mut self, event: Event) {
        match event {
            Event::Status(Ok(value)) => {
                self.server_alive = true;
                if !self.target_from_status
                    && let Some(first) = value["projects"].as_array().and_then(|a| a.first())
                {
                    if let Some(id) = first["id"].as_str() {
                        self.project = id.to_string();
                    }
                    if let Some(art) = first["artifacts"].as_array().and_then(|a| a.first())
                        && let Some(art) = art.as_str()
                    {
                        self.artifact = art.to_string();
                    }
                    self.target_from_status = true;
                }
                self.status = Some(value);
            }
            Event::Status(Err(e)) => {
                if matches!(e, FacadeError::ServerDied(_)) {
                    self.server_alive = false;
                }
                self.push_log(format!("status failed: {e}"));
            }
            Event::Tools(Ok(tools)) => {
                self.tool_count = Some(tools.len());
            }
            Event::Tools(Err(e)) => {
                self.push_log(format!("tools failed: {e}"));
            }
            Event::Gates(Ok(checklist)) => {
                self.pipeline.gates_authorized = checklist.all_green();
                self.push_log(format!(
                    "certification loaded: {} ({}/{} gates pass)",
                    checklist.verdict,
                    checklist.passed().0,
                    checklist.passed().1
                ));
                self.gates = Some(checklist);
            }
            Event::Gates(Err(e)) => {
                self.push_log(format!("certification failed: {e}"));
            }
            Event::StepStarted { index } => {
                let label = self
                    .pipeline
                    .steps
                    .get(index)
                    .map(|s| s.kind.tool().to_string());
                if let Some(label) = label {
                    self.push_log(format!("step started: {label}"));
                }
            }
            Event::StepLog { index, line } => {
                if let Some(step) = self.pipeline.steps.get_mut(index) {
                    step.server_log_lines += 1;
                }
                self.push_log(line);
            }
            Event::StepFinished { index, result } => match result {
                Ok(_) => {
                    let label = self.pipeline.steps.get(index).map(|s| s.kind.tool());
                    if let Some(step) = self.pipeline.steps.get_mut(index) {
                        step.elapsed = step.started.map(|s| s.elapsed());
                        step.state = StepState::Done;
                    }
                    if let Some(label) = label {
                        self.push_log(format!("step done: {label}"));
                    }
                    self.advance();
                }
                Err(FacadeError::ServerDied(detail)) => {
                    self.mark_server_died(index, &detail);
                }
                Err(FacadeError::Message(message)) => {
                    if let Some(step) = self.pipeline.steps.get_mut(index) {
                        step.state = StepState::Failed(message.clone());
                    }
                    for step in &mut self.pipeline.steps {
                        if step.state == StepState::Pending {
                            step.state = StepState::Skipped;
                        }
                    }
                    self.pipeline.state = PipelineState::Failed;
                    self.push_log(format!("step failed: {message}"));
                }
            },
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::{FakeFacade, RecordedCalls};
    use crate::pipeline::StepKind;
    use crate::triggers::default_rules;

    fn green_certify() -> Value {
        json!({
            "project": "demo", "channel": "internal", "verdict": "certified",
            "gates": [
                {"name": "signature", "result": "pass", "detail": "signed"},
                {"name": "alignment", "result": "pass", "detail": "16k aligned"},
            ],
        })
    }

    fn app_with(fake: FakeFacade) -> (App, RecordedCalls) {
        let calls = fake.calls.clone();
        (
            App::new(Box::new(fake), TriggerEngine::new(default_rules())),
            calls,
        )
    }

    fn recorded(calls: &RecordedCalls) -> Vec<String> {
        calls
            .lock()
            .map(|g| g.iter().map(|(t, _)| t.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn plan_mode_makes_no_call_before_approval() {
        let (mut app, calls) = app_with(FakeFacade::new());
        app.pump_until_settled();
        app.run_pipeline();
        app.pump_until_settled();
        // Plan mode stages the first step; the facade must have
        // seen NO workflow call at all yet.
        let workflow_calls: Vec<String> = recorded(&calls)
            .into_iter()
            .filter(|t| t != "slipway_status")
            .collect();
        assert!(
            workflow_calls.is_empty(),
            "calls before approval: {workflow_calls:?}"
        );
        assert_eq!(app.pipeline.state, PipelineState::AwaitingApproval);
        // Approval releases exactly that one step, then the next
        // step stages again.
        app.approve();
        app.pump_until_settled();
        assert_eq!(recorded(&calls), vec!["slipway_build"]);
        assert_eq!(app.pipeline.state, PipelineState::AwaitingApproval);
    }

    #[test]
    fn plan_mode_reject_makes_no_call_at_all() {
        let (mut app, calls) = app_with(FakeFacade::new());
        app.pump_until_settled();
        app.run_pipeline();
        app.pump_until_settled();
        app.reject();
        app.pump_until_settled();
        assert_eq!(app.pipeline.state, PipelineState::Aborted);
        assert!(recorded(&calls).is_empty());
    }

    #[test]
    fn auto_mode_runs_build_then_stops_before_sign() {
        let (mut app, calls) = app_with(FakeFacade::new());
        app.pump_until_settled();
        app.mode = Mode::Auto;
        app.run_pipeline();
        app.pump_until_settled();
        // Build ran freely; sign (consequential) did NOT run —
        // the pipeline is stopped at its approval gate.
        assert_eq!(recorded(&calls), vec!["slipway_build"]);
        assert_eq!(app.pipeline.state, PipelineState::AwaitingApproval);
        assert_eq!(
            app.pipeline.steps[1].state,
            StepState::AwaitingApproval,
            "sign must be the staged step"
        );
        assert_eq!(app.pipeline.steps[1].kind, StepKind::Sign);
    }

    #[test]
    fn auto_mode_with_green_gates_runs_the_full_pipeline() {
        let mut fake = FakeFacade::new();
        fake.queue_call_result("slipway_certify", Ok(green_certify()));
        let (mut app, calls) = app_with(fake);
        app.pump_until_settled();
        app.request_gates();
        app.pump_until_settled();
        assert!(app.pipeline.gates_authorized);
        app.mode = Mode::Auto;
        app.run_pipeline();
        app.pump_until_settled();
        assert_eq!(app.pipeline.state, PipelineState::Completed);
        assert_eq!(
            recorded(&calls),
            vec![
                "slipway_certify",
                "slipway_build",
                "slipway_sign",
                "slipway_verify",
                "slipway_install",
                "slipway_release",
            ]
        );
        // And the green completion fired the bloch rule.
        assert_eq!(app.triggers.firings().len(), 1);
        assert_eq!(app.triggers.firings()[0].target_mcp, "bloch");
    }

    #[test]
    fn server_death_mid_pipeline_marks_unknown_never_success() {
        let mut fake = FakeFacade::new();
        fake.die_on_call = Some("slipway_sign".to_string());
        let (mut app, _calls) = app_with(fake);
        app.pump_until_settled();
        app.mode = Mode::Auto;
        app.run_pipeline();
        app.pump_until_settled();
        // Build finished; the pipeline stopped before sign, a
        // human approved it, and the server died on the call.
        app.approve();
        app.pump_until_settled();
        assert_eq!(app.pipeline.state, PipelineState::Failed);
        assert!(!app.server_alive);
        assert_eq!(app.pipeline.steps[0].state, StepState::Done);
        assert!(
            matches!(app.pipeline.steps[1].state, StepState::Unknown(_)),
            "sign state: {:?}",
            app.pipeline.steps[1].state
        );
        for step in &app.pipeline.steps[2..] {
            assert!(
                matches!(step.state, StepState::Unknown(_)),
                "later step {:?} must be unknown, got {:?}",
                step.kind,
                step.state
            );
        }
        assert!(!app.pipeline.all_done());
    }

    #[test]
    fn step_failure_marks_failed_and_skips_the_rest() {
        let mut fake = FakeFacade::new();
        fake.queue_call_result(
            "slipway_build",
            Err(FacadeError::Message("gradle exited 1".into())),
        );
        let (mut app, _calls) = app_with(fake);
        app.pump_until_settled();
        app.mode = Mode::Auto;
        app.run_pipeline();
        app.pump_until_settled();
        assert_eq!(app.pipeline.state, PipelineState::Failed);
        assert!(matches!(app.pipeline.steps[0].state, StepState::Failed(_)));
        assert_eq!(app.pipeline.steps[1].state, StepState::Skipped);
    }

    #[test]
    fn non_green_certification_does_not_authorize_or_fire() {
        let mut fake = FakeFacade::new();
        fake.queue_call_result(
            "slipway_certify",
            Ok(json!({
                "project": "demo", "channel": "internal", "verdict": "refused",
                "gates": [{"name": "signature", "result": "fail",
                           "detail": "unsigned"}],
            })),
        );
        let (mut app, _calls) = app_with(fake);
        app.pump_until_settled();
        app.request_gates();
        app.pump_until_settled();
        assert!(!app.pipeline.gates_authorized);
        assert!(app.triggers.firings().is_empty());
    }

    #[test]
    fn empty_gate_list_is_never_all_green() {
        let checklist = GateChecklist::parse(&json!({"gates": []})).expect("parse");
        assert!(!checklist.all_green());
        assert!(GateChecklist::parse(&json!({"nope": 1})).is_err());
    }
}
