//! `slipway-tui` — thin binary: parse args, build the real
//! facade, run.
//!
//! Facade choice (exactly one, no silent fallback):
//! - `--fake` — the scripted in-memory facade (demos, smoke
//!   tests; it performs no real work at all).
//! - `--config <path>` — the library facade over the member's
//!   own `slipway` library with that config.
//! - `--stdio <binary> [--config <path>]` — the MCP stdio
//!   client facade spawning the member's server binary.
//!
//! Modes: full-screen ratatui on a TTY; a non-TTY line mode
//! (also forced by `--line`) sharing the same execute path.
//! No real signing, installing, or releasing happens through
//! the fake facade, and the consequential-step approval rules
//! in `pipeline` govern every facade.

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;
use std::time::Duration;

use serde_json::json;

use slipway_tui::app::App;
use slipway_tui::facade::{FacadeError, FakeFacade, LibFacade, SlipwayFacade, StdioFacade};
use slipway_tui::pipeline::Mode;
use slipway_tui::screens::{Screen, header_lines, screen_lines};
use slipway_tui::triggers::{TriggerEngine, default_rules, load_rules};

fn build_facade(args: &[String]) -> Result<Box<dyn SlipwayFacade>, String> {
    let arg = |name: &str| -> Option<String> {
        args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone())
    };
    if args.iter().any(|a| a == "--fake") {
        let mut fake = FakeFacade::new();
        // Scripted demo data: an all-green certification and a
        // build that reports two log lines. Nothing real runs.
        for _ in 0..4 {
            fake.queue_call_result(
                "slipway_certify",
                Ok(json!({
                    "project": "demo", "channel": "internal",
                    "verdict": "certified",
                    "gates": [
                        {"name": "signature", "result": "pass",
                         "detail": "scripted demo gate"},
                        {"name": "alignment", "result": "pass",
                         "detail": "scripted demo gate"},
                    ],
                })),
            );
        }
        fake.queue_call_result(
            "slipway_build",
            Ok(json!({"ok": true,
                      "log": ["demo build: step 1 reported by server",
                              "demo build: step 2 reported by server"]})),
        );
        return Ok(Box::new(fake));
    }
    if let Some(binary) = arg("--stdio") {
        let mut argv = vec![binary];
        if let Some(config) = arg("--config") {
            argv.push("--config".to_string());
            argv.push(config);
        }
        return StdioFacade::spawn(&argv)
            .map(|f| Box::new(f) as Box<dyn SlipwayFacade>)
            .map_err(|e: FacadeError| e.to_string());
    }
    if let Some(config) = arg("--config") {
        return LibFacade::load(std::path::Path::new(&config))
            .map(|f| Box::new(f) as Box<dyn SlipwayFacade>)
            .map_err(|e: FacadeError| e.to_string());
    }
    Err("no facade selected: pass --fake, --config <path>, or --stdio <binary>".to_string())
}

fn build_triggers(args: &[String]) -> Result<TriggerEngine, String> {
    let explicit = args
        .windows(2)
        .find(|w| w[0] == "--triggers")
        .map(|w| w[1].clone());
    let path = explicit.or_else(|| {
        let candidate = std::path::Path::new("triggers.toml");
        candidate.exists().then(|| "triggers.toml".to_string())
    });
    match path {
        Some(path) => {
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
            Ok(TriggerEngine::new(load_rules(&text)?))
        }
        None => Ok(TriggerEngine::new(default_rules())),
    }
}

fn render_text(app: &App) -> String {
    let mut out = header_lines(app);
    out.extend(screen_lines(app, app.screen, std::time::Instant::now()));
    out.join("\n")
}

fn handle_line(app: &mut App, line: &str) -> Option<String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    match parts.as_slice() {
        ["quit"] | ["exit"] => return None,
        ["overview"] => app.screen = Screen::Overview,
        ["workflows"] => app.screen = Screen::Workflows,
        ["gates"] => app.screen = Screen::Gates,
        ["log"] => app.screen = Screen::Log,
        ["fleet"] => app.screen = Screen::Fleet,
        ["status"] => {
            app.request_status();
            app.pump_until_settled();
            app.screen = Screen::Overview;
        }
        ["tools"] => {
            app.request_tools();
            app.pump_until_settled();
            return Some(format!("tools: {}", app.tool_count.unwrap_or(0)));
        }
        ["certify"] => {
            app.request_gates();
            app.pump_until_settled();
            app.screen = Screen::Gates;
        }
        ["mode", "plan"] => app.mode = Mode::Plan,
        ["mode", "auto"] => app.mode = Mode::Auto,
        ["mode"] => app.toggle_mode(),
        ["run"] => {
            app.run_pipeline();
            app.pump_until_settled();
        }
        ["approve"] => {
            app.approve();
            app.pump_until_settled();
        }
        ["reject"] => app.reject(),
        ["target", project, channel, artifact] => {
            app.project = (*project).to_string();
            app.channel = (*channel).to_string();
            app.artifact = (*artifact).to_string();
        }
        ["tick"] => {
            app.tick();
        }
        [] => {}
        other => {
            return Some(format!("unknown command: {}", other.join(" ")));
        }
    }
    Some(render_text(app))
}

fn run_line_mode(app: &mut App) -> ExitCode {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        match handle_line(app, &line) {
            Some(text) => {
                let _ = writeln!(stdout, "{text}");
                let _ = stdout.flush();
            }
            None => return ExitCode::SUCCESS,
        }
    }
    ExitCode::SUCCESS
}

fn run_fullscreen(app: &mut App) -> ExitCode {
    use crossterm::event::{Event, KeyCode, KeyEventKind};
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };
    use ratatui::Terminal;
    use ratatui::backend::CrosstermBackend;

    if enable_raw_mode().is_err() {
        eprintln!("slipway-tui: cannot enter raw mode");
        return ExitCode::from(1);
    }
    let mut stdout = std::io::stdout();
    let _ = crossterm::execute!(stdout, EnterAlternateScreen);
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(_) => {
            let _ = disable_raw_mode();
            return ExitCode::from(1);
        }
    };
    let mut code = ExitCode::SUCCESS;
    loop {
        app.tick();
        if terminal
            .draw(|f| slipway_tui::panels::draw(f, app))
            .is_err()
        {
            code = ExitCode::from(1);
            break;
        }
        match crossterm::event::poll(Duration::from_millis(250)) {
            Ok(true) => {
                if let Ok(Event::Key(key)) = crossterm::event::read()
                    && key.kind == KeyEventKind::Press
                {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Tab => app.screen = app.screen.next(),
                        KeyCode::Char('1') => app.screen = Screen::Overview,
                        KeyCode::Char('2') => app.screen = Screen::Workflows,
                        KeyCode::Char('3') => app.screen = Screen::Gates,
                        KeyCode::Char('4') => app.screen = Screen::Log,
                        KeyCode::Char('5') => app.screen = Screen::Fleet,
                        KeyCode::Char('m') => app.toggle_mode(),
                        KeyCode::Char('r') => app.run_pipeline(),
                        KeyCode::Char('a') => app.approve(),
                        KeyCode::Char('x') => app.reject(),
                        KeyCode::Char('g') => app.request_gates(),
                        KeyCode::Char('s') => app.request_status(),
                        _ => {}
                    }
                }
            }
            Ok(false) => {}
            Err(_) => {
                code = ExitCode::from(1);
                break;
            }
        }
    }
    let _ = disable_raw_mode();
    let _ = crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen);
    code
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let facade = match build_facade(&args) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("slipway-tui: {e}");
            return ExitCode::from(2);
        }
    };
    let triggers = match build_triggers(&args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("slipway-tui: {e}");
            return ExitCode::from(2);
        }
    };
    let mut app = App::new(facade, triggers);
    app.pump_until_settled();
    if args.iter().any(|a| a == "--line") || !std::io::stdout().is_terminal() {
        run_line_mode(&mut app)
    } else {
        run_fullscreen(&mut app)
    }
}
