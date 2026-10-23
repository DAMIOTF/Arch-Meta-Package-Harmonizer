use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph};

use crate::tui::app::{App, ScanPhase};
use crate::tui::widgets;

pub fn scan_strip(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let theme = &app.theme;
    let width = area.width;
    let percent = app.scan.percent().unwrap_or(0);
    let spinner = widgets::spinner(app.tick);

    let stage = format!(
        "{} {}",
        spinner,
        widgets::truncate(&app.scan.stage, width.saturating_sub(20))
    );
    let detail = if app.scan.detail.is_empty() {
        app.scan
            .percent()
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "waiting for progress".to_string())
    } else {
        widgets::truncate(&app.scan.detail, 30)
    };

    let left = Line::from(vec![
        Span::styled(
            stage,
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(detail, Style::default().fg(theme.muted)),
    ]);

    frame.render_widget(
        Paragraph::new(left),
        Rect {
            x: area.x,
            y: area.y,
            width: area.width.saturating_sub(10),
            height: 1,
        },
    );

    let elapsed = widgets::format_elapsed(app.scan.elapsed().as_secs_f64());
    frame.render_widget(
        Paragraph::new(Span::styled(elapsed, Style::default().fg(theme.muted)))
            .alignment(Alignment::Right),
        Rect {
            x: area.x,
            y: area.y,
            width,
            height: 1,
        },
    );

    if area.height > 1 {
        let gauge_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        };
        let label = if app.scan.phase == ScanPhase::Starting || app.scan.total == 0 {
            " waiting for scanner ".to_string()
        } else {
            format!(
                " {} / {} · {}% ",
                app.scan.done.min(app.scan.total),
                app.scan.total,
                percent
            )
        };
        let gauge = Gauge::default()
            .ratio(percent as f64 / 100.0)
            .label(Span::styled(
                label,
                Style::default().fg(theme.text).bg(theme.bg),
            ))
            .gauge_style(
                Style::default()
                    .fg(theme.accent)
                    .bg(theme.surface_alt)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_widget(gauge, gauge_area);
    }
}
