use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph};

use crate::tui::app::App;
use crate::tui::widgets;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    if area.height == 0 {
        return;
    }

    let theme = app.theme;
    let metrics = &app.inventory.metrics;

    if metrics.total_packages == 0 {
        let block = widgets::panel("Harmony", &theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        widgets::empty_state(
            frame,
            inner,
            "No packages to harmonize",
            "Send a snapshot from the scanner to begin.",
            &theme,
        );
        return;
    }

    let sections = Layout::vertical([
        Constraint::Length(7),
        Constraint::Length(6),
        Constraint::Min(0),
    ])
    .spacing(1)
    .split(area);

    render_hero(frame, sections[0], app);
    render_balance(frame, sections[1], app);

    if sections[2].height >= 3 {
        render_recommendations(frame, sections[2], app);
    }
}

fn render_hero(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let metrics = &app.inventory.metrics;
    let score = metrics.harmony;
    let color = theme.score_color(score);
    let block = widgets::accent_panel("Harmony index", theme, color);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let big_width = widgets::big_number_width(score as u32).saturating_add(2);
    let columns =
        Layout::horizontal([Constraint::Length(big_width), Constraint::Min(0)]).split(inner);

    let number_lines = widgets::big_number_lines(score as u32, color);
    frame.render_widget(Paragraph::new(number_lines), columns[0]);

    if columns[1].width == 0 {
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(columns[1]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                widgets::score_word(score).to_ascii_uppercase(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  system balance", Style::default().fg(theme.muted)),
        ])),
        rows[0],
    );

    let gauge = Gauge::default()
        .percent(score as u16)
        .label(Span::styled(
            format!(" {score}% "),
            Style::default()
                .fg(theme.text)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ))
        .gauge_style(
            Style::default()
                .fg(color)
                .bg(theme.surface_alt)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(gauge, rows[1]);

    let attention = app
        .inventory
        .checks
        .iter()
        .filter(|check| check.status.is_attention())
        .count();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("health {}", metrics.health),
                Style::default().fg(theme.score_color(metrics.health)),
            ),
            Span::styled(
                format!(
                    "  ·  {} signal{}  ·  {} foreign",
                    attention,
                    if attention == 1 { "" } else { "s" },
                    metrics.foreign
                ),
                Style::default().fg(theme.muted),
            ),
        ])),
        rows[2],
    );

    if rows[3].height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("footprint ", Style::default().fg(theme.muted)),
                Span::styled(
                    format!("{}/100", metrics.footprint_score),
                    Style::default().fg(theme.score_color(metrics.footprint_score)),
                ),
                Span::styled(
                    format!(
                        "  ·  {} installed",
                        crate::tui::model::format_size(metrics.total_size)
                    ),
                    Style::default().fg(theme.muted),
                ),
            ])),
            rows[3],
        );
    }
}

fn render_balance(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let metrics = &app.inventory.metrics;
    let block = widgets::panel("Balance", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let bar_width = inner.width.saturating_sub(21).max(4);
    let lines = vec![
        widgets::meter_line("Structure", metrics.structure_score, bar_width, theme),
        widgets::meter_line("Dependencies", metrics.dependency_score, bar_width, theme),
        widgets::meter_line("Cleanliness", metrics.cleanliness_score, bar_width, theme),
        widgets::meter_line("Provenance", metrics.provenance_score, bar_width, theme),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_recommendations(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = widgets::panel("Recommendations", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut ordered = app
        .inventory
        .checks
        .iter()
        .filter(|check| check.status.is_attention())
        .collect::<Vec<_>>();
    if ordered.is_empty() {
        ordered = app
            .inventory
            .checks
            .iter()
            .filter(|check| check.status == crate::tui::metrics::CheckStatus::Info)
            .collect::<Vec<_>>();
    }

    let mut lines = Vec::new();
    for check in ordered.into_iter().take(inner.height as usize) {
        let color = theme.status_color(check.status);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", check.status.label()),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                widgets::truncate(&check.title, inner.width.saturating_sub(6)),
                Style::default().fg(theme.text),
            ),
        ]));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "Everything is in balance.",
            Style::default()
                .fg(theme.positive)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "No cleanup or dependency action is suggested.",
            Style::default().fg(theme.muted),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}
