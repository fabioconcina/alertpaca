use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::checks::{CheckStatus, Section};
use crate::tui::App;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const GREEN: Color = Color::Indexed(82);
const YELLOW: Color = Color::Indexed(220);
const RED: Color = Color::Indexed(196);
const DIM: Color = Color::Indexed(243);

fn status_icon(status: CheckStatus) -> Span<'static> {
    match status {
        CheckStatus::Ok => Span::styled(" ✓ ", Style::default().fg(GREEN)),
        CheckStatus::Warning => Span::styled(" ⚠ ", Style::default().fg(YELLOW)),
        CheckStatus::Critical => Span::styled(" ✗ ", Style::default().fg(RED)),
        CheckStatus::Skipped => Span::styled(" — ", Style::default().fg(DIM)),
    }
}

fn summary_style(status: CheckStatus) -> Style {
    match status {
        CheckStatus::Ok => Style::default(),
        CheckStatus::Warning => Style::default().fg(YELLOW),
        CheckStatus::Critical => Style::default().fg(RED),
        CheckStatus::Skipped => Style::default().fg(DIM),
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    if app.splash {
        draw_splash(f, area);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(1),   // body
        Constraint::Length(1), // footer
    ])
    .split(area);

    draw_header(f, chunks[0], app);
    draw_body(f, chunks[1], app);
    draw_footer(f, chunks[2], app);
}

fn draw_splash(f: &mut Frame, area: Rect) {
    let logo_lines = [
        "▄▀█ █   █▀▀ █▀█ ▀█▀ █▀█ ▄▀█ █▀▀ ▄▀█",
        "█▀█ █▄▄ ██▄ ██▀  █  █▀▀ █▀█ █▄▄ █▀█",
    ];
    let sep = "───────────────────────────────────────";
    let sub = format!("Server health checker  ·  v{}", VERSION);

    let content_height: u16 = 4; // 2 logo + 1 sep + 1 sub
    let content_width = logo_lines[0].chars().count() as u16;

    let top = area.height.saturating_sub(content_height) / 2;
    let left = area.width.saturating_sub(content_width) / 2;

    let logo_style = Style::default().fg(GREEN);
    let sep_style = Style::default().fg(DIM);

    let lines = vec![
        Line::styled(logo_lines[0], logo_style),
        Line::styled(logo_lines[1], logo_style),
        Line::styled(sep, sep_style),
        Line::styled(sub, Style::default().fg(Color::White)),
    ];

    let splash_area = Rect::new(left, top, content_width, content_height);
    f.render_widget(Paragraph::new(lines), splash_area);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let timestamp = match &app.last_check {
        Some(t) => format!("last check: {}", t.format("%H:%M:%S")),
        None if app.checking => "checking...".to_string(),
        None => "—".to_string(),
    };

    let title = format!("alertpaca v{}", VERSION);
    let padding = area
        .width
        .saturating_sub(title.len() as u16 + timestamp.len() as u16) as usize;

    let header = Line::from(vec![
        Span::styled(
            title,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" ".repeat(padding)),
        Span::styled(timestamp, Style::default().fg(DIM)),
    ]);

    let sep = Line::from(Span::styled(
        "─".repeat(area.width as usize),
        Style::default().fg(DIM),
    ));

    let widget = Paragraph::new(vec![header, sep]);
    f.render_widget(widget, area);
}

fn draw_body(f: &mut Frame, area: Rect, app: &mut App) {
    let mut lines: Vec<Line> = Vec::new();

    if app.results.is_empty() && app.checking {
        lines.push(Line::from(Span::styled(
            "  Running checks...",
            Style::default().fg(DIM),
        )));
    }

    let mut current_section: Option<Section> = None;

    for result in &app.results {
        if current_section != Some(result.section) {
            if current_section.is_some() {
                lines.push(Line::raw("")); // blank line between sections
            }
            lines.push(Line::from(Span::styled(
                format!(" {}", result.section.label()),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )));
            current_section = Some(result.section);
        }

        // Pad name to 16 chars for alignment
        let name = format!("{:<16}", result.name);
        let line = Line::from(vec![
            status_icon(result.status),
            Span::styled(name, Style::default().fg(Color::White)),
            Span::styled(result.summary.clone(), summary_style(result.status)),
        ]);
        lines.push(line);
    }

    app.content_height = lines.len() as u16;
    let widget = Paragraph::new(lines).scroll((app.scroll_offset, 0));
    f.render_widget(widget, area);
}

fn draw_footer(f: &mut Frame, area: Rect, _app: &App) {
    let footer = Line::from(vec![
        Span::styled(" q", Style::default().fg(Color::White)),
        Span::styled(": quit  ", Style::default().fg(DIM)),
        Span::styled("r", Style::default().fg(Color::White)),
        Span::styled(": refresh  ", Style::default().fg(DIM)),
        Span::styled("↑↓", Style::default().fg(Color::White)),
        Span::styled(": scroll", Style::default().fg(DIM)),
    ]);
    f.render_widget(Paragraph::new(footer), area);
}
