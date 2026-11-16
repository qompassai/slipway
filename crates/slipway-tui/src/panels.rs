//! Pure ratatui widget constructors. Panels take line vectors
//! (from [`crate::screens`]) and build widgets; they own the
//! truncation bounds so no render path can emit an unbounded
//! line or an unbounded screen.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::screens;

/// Maximum characters in one rendered line.
pub const LINE_CHARS_MAX: usize = 160;
/// Maximum body lines rendered at once.
pub const BODY_LINES_MAX: usize = 200;

/// Truncate one line to [`LINE_CHARS_MAX`] chars (char-boundary
/// safe), marking the cut.
pub fn truncate_line(line: &str) -> String {
    if line.chars().count() <= LINE_CHARS_MAX {
        return line.to_string();
    }
    let mut out: String = line.chars().take(LINE_CHARS_MAX - 1).collect();
    out.push('…');
    out
}

/// Build the body widget for a screen from its lines.
pub fn body_widget(lines: &[String], title: &str) -> Paragraph<'static> {
    let rendered: Vec<Line<'static>> = lines
        .iter()
        .take(BODY_LINES_MAX)
        .map(|l| Line::from(truncate_line(l)))
        .collect();
    Paragraph::new(rendered)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title.to_string()),
        )
        .wrap(Wrap { trim: false })
}

/// Build the header widget (member line + screen tabs).
pub fn header_widget(lines: &[String]) -> Paragraph<'static> {
    let rendered: Vec<Line<'static>> = lines.iter().map(|l| Line::from(truncate_line(l))).collect();
    Paragraph::new(rendered)
}

/// Draw the whole app: header, then the current screen's body.
pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(frame.area());
    frame.render_widget(header_widget(&screens::header_lines(app)), chunks[0]);
    let now = std::time::Instant::now();
    let lines = screens::screen_lines(app, app.screen, now);
    frame.render_widget(body_widget(&lines, app.screen.title()), chunks[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::FakeFacade;
    use crate::triggers::{TriggerEngine, default_rules};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn truncation_bounds_long_lines() {
        let long = "x".repeat(LINE_CHARS_MAX + 50);
        let out = truncate_line(&long);
        assert_eq!(out.chars().count(), LINE_CHARS_MAX);
        assert!(out.ends_with('…'));
        assert_eq!(truncate_line("short"), "short");
    }

    #[test]
    fn first_screen_renders_on_test_backend() {
        let app = App::new(
            Box::new(FakeFacade::new()),
            TriggerEngine::new(default_rules()),
        );
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|f| draw(f, &app)).expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let text: String = buffer
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert!(text.contains("slipway-tui"), "rendered: {text}");
        assert!(text.contains("Overview"), "rendered: {text}");
    }
}
