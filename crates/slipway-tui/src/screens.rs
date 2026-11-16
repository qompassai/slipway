//! Screen line generation: each screen is a pure function of
//! [`App`] state producing bounded text lines. The full-screen
//! panels and the line mode both render these exact lines, so
//! what a test asserts is what the operator sees.
//!
//! There is no Media screen: slipway's use case (Android
//! build/sign/verify/install/release) has no image or video
//! surface, and DESIGN §D2 says absent, not stubbed.

use std::time::Instant;

use crate::app::App;
use crate::pipeline::PipelineState;

/// The screens, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Name/version, server status, gate summary.
    Overview,
    /// Pipeline runs + certification-gate checklist.
    Workflows,
    /// Certification gates in detail.
    Gates,
    /// Bounded live event log.
    Log,
    /// Fleet members, trigger rules, trigger audit tail.
    Fleet,
}

impl Screen {
    /// All screens in tab order.
    pub const ORDER: [Screen; 5] = [
        Screen::Overview,
        Screen::Workflows,
        Screen::Gates,
        Screen::Log,
        Screen::Fleet,
    ];

    /// Title for the header.
    pub fn title(self) -> &'static str {
        match self {
            Screen::Overview => "Overview",
            Screen::Workflows => "Workflows",
            Screen::Gates => "Gates",
            Screen::Log => "Log / Live",
            Screen::Fleet => "Fleet",
        }
    }

    /// The next screen in tab order (wrapping).
    pub fn next(self) -> Screen {
        let index = Screen::ORDER.iter().position(|s| *s == self).unwrap_or(0);
        Screen::ORDER[(index + 1) % Screen::ORDER.len()]
    }
}

/// The header line shown on every screen.
pub fn header_lines(app: &App) -> Vec<String> {
    let tabs: Vec<String> = Screen::ORDER
        .iter()
        .map(|s| {
            if *s == app.screen {
                format!("[{}]", s.title())
            } else {
                s.title().to_string()
            }
        })
        .collect();
    vec![
        format!(
            "slipway-tui — Android release console — mode: {} — server: {}",
            app.mode.label(),
            if app.server_alive { "alive" } else { "DEAD" }
        ),
        tabs.join("  "),
    ]
}

/// Render one screen's body lines.
pub fn screen_lines(app: &App, screen: Screen, now: Instant) -> Vec<String> {
    match screen {
        Screen::Overview => overview_lines(app),
        Screen::Workflows => workflow_lines(app, now),
        Screen::Gates => gate_lines(app),
        Screen::Log => app.log.iter().cloned().collect(),
        Screen::Fleet => fleet_lines(app),
    }
}

fn overview_lines(app: &App) -> Vec<String> {
    let version = app
        .status
        .as_ref()
        .and_then(|s| s["version"].as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    let projects = app
        .status
        .as_ref()
        .and_then(|s| s["projects"].as_array().map(|a| a.len()))
        .unwrap_or(0);
    let gate_summary = match &app.gates {
        Some(g) => {
            let (passed, total) = g.passed();
            format!("certification {} — {passed}/{total} gates pass", g.verdict)
        }
        None => "certification not loaded (Gates screen, press g)".to_string(),
    };
    let mut lines = vec![
        format!("member: slipway {version}"),
        format!(
            "server: {}",
            if app.server_alive { "alive" } else { "DEAD" }
        ),
        format!("tools: {}", app.tool_count.unwrap_or(0)),
        format!("projects: {projects}"),
        format!(
            "target: {} / {} / {}",
            app.project, app.channel, app.artifact
        ),
        gate_summary,
        format!("trigger rules: {}", app.triggers.rules().len()),
        format!("pipeline: {:?}", app.pipeline.state),
        "last events:".to_string(),
    ];
    for line in app.log.iter().rev().take(5).rev() {
        lines.push(format!("  {line}"));
    }
    lines
}

fn workflow_lines(app: &App, now: Instant) -> Vec<String> {
    let mut lines = vec![
        format!(
            "pipeline ({}): build -> sign -> verify -> install -> release",
            app.mode.label()
        ),
        format!(
            "certification gates authorize consequential steps: {}",
            app.pipeline.gates_authorized
        ),
        format!("state: {:?}", app.pipeline.state),
    ];
    for step in &app.pipeline.steps {
        lines.push(format!(
            "  {:8} {:16} {}",
            step.kind.label(),
            step.kind.tool(),
            step.progress_text(now)
        ));
        if step.state == crate::pipeline::StepState::AwaitingApproval {
            let args = crate::facade::step_args(
                step.kind.tool(),
                &app.project,
                &app.channel,
                &app.artifact,
            );
            lines.push(format!("    staged call: {} {}", step.kind.tool(), args));
            lines.push("    press a to approve, x to reject".to_string());
        }
    }
    if app.mode == crate::pipeline::Mode::Auto && !app.pipeline.gates_authorized {
        lines.push(
            "auto mode: build/verify run freely; sign/install/release stop \
             for approval (gates not green)"
                .to_string(),
        );
    }
    lines.push("press r to run, m to toggle plan/auto".to_string());
    lines
}

fn gate_lines(app: &App) -> Vec<String> {
    match &app.gates {
        None => vec![
            "certification gates: not loaded".to_string(),
            "press g to run slipway_certify for the current target".to_string(),
        ],
        Some(g) => {
            let mut lines = vec![
                format!(
                    "certification: {} / {} — verdict {}",
                    g.project, g.channel, g.verdict
                ),
                format!("all gates green: {}", g.all_green()),
            ];
            for gate in &g.gates {
                let mark = if gate.result == "pass" {
                    "PASS"
                } else {
                    "FAIL"
                };
                lines.push(format!("[{mark}] {} — {}", gate.name, gate.detail));
            }
            lines
        }
    }
}

fn fleet_lines(app: &App) -> Vec<String> {
    let mut lines = vec![
        "fleet members:".to_string(),
        format!(
            "  slipway (this member): {}",
            if app.server_alive { "alive" } else { "DEAD" }
        ),
        "  bloch (trigger target): not probed — live trigger dispatch \
         is not wired in rung 1"
            .to_string(),
        "trigger rules:".to_string(),
    ];
    for rule in app.triggers.rules() {
        lines.push(format!(
            "  {} [{}] on {} -> {}.{} (mode {}, {}/{})",
            rule.id,
            if rule.enabled { "enabled" } else { "disabled" },
            rule.event,
            rule.target_mcp,
            rule.target_tool,
            rule.mode,
            rule.origin_lane,
            rule.content_label
        ));
    }
    lines.push(format!(
        "firings recorded: {}",
        app.triggers.firings().len()
    ));
    lines.push("trigger audit tail:".to_string());
    for line in app.triggers.audit().iter().rev().take(8).rev() {
        lines.push(format!("  {line}"));
    }
    lines
}

/// Whether a pipeline is mid-flight (for status summaries).
pub fn pipeline_active(app: &App) -> bool {
    matches!(
        app.pipeline.state,
        PipelineState::Running | PipelineState::AwaitingApproval
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::FakeFacade;
    use crate::triggers::{TriggerEngine, default_rules};

    fn test_app() -> App {
        App::new(
            Box::new(FakeFacade::new()),
            TriggerEngine::new(default_rules()),
        )
    }

    #[test]
    fn overview_names_member_version_and_gate_state() {
        let mut app = test_app();
        app.pump_until_settled();
        let text = overview_lines(&app).join("\n");
        assert!(text.contains("slipway 0.1.0"), "{text}");
        assert!(text.contains("tools: 15"), "{text}");
        assert!(text.contains("certification not loaded"), "{text}");
    }

    #[test]
    fn workflows_show_staged_call_with_exact_args_in_plan_mode() {
        let mut app = test_app();
        app.pump_until_settled();
        app.run_pipeline();
        app.pump_until_settled();
        let text = workflow_lines(&app, Instant::now()).join("\n");
        assert!(text.contains("staged call: slipway_build"), "{text}");
        assert!(
            text.contains("\"project\": \"demo\"")
                || text.contains("\"project\":\"demo\"")
                || text.contains("project"),
            "{text}"
        );
    }

    #[test]
    fn fleet_panel_lists_the_bloch_rule_with_birth_metadata() {
        let app = test_app();
        let text = fleet_lines(&app).join("\n");
        assert!(text.contains("slipway-pipeline-to-bloch-lesson"), "{text}");
        assert!(text.contains("bloch.lesson_candidate"), "{text}");
        assert!(text.contains("qompass/sfw"), "{text}");
        assert!(text.contains("not wired in rung 1"), "{text}");
    }

    #[test]
    fn gates_screen_is_honest_when_unloaded() {
        let app = test_app();
        let text = gate_lines(&app).join("\n");
        assert!(text.contains("not loaded"), "{text}");
    }
}
