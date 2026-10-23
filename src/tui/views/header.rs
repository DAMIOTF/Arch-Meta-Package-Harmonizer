use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::tui::app::{App, ScanPhase};
use crate::tui::widgets;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }

    let theme = &app.theme;
    frame.render_widget(Block::default().style(theme.mantle_style()), area);

    let (status_text, status_color) = match app.scan.phase {
        ScanPhase::Starting => (
            format!("{} STARTING", widgets::spinner(app.tick)),
            theme.warning,
        ),
        ScanPhase::Running => {
            let percent = app
                .scan
                .percent()
                .map(|value| format!(" {value}%"))
                .unwrap_or_default();
            (
                format!("{} SCANNING{percent}", widgets::spinner(app.tick)),
                theme.info,
            )
        }
        ScanPhase::Complete => {
            if app.sample {
                ("SAMPLE PREVIEW".to_string(), theme.warning)
            } else {
                ("SCAN COMPLETE".to_string(), theme.positive)
            }
        }
        ScanPhase::Failed => ("SCAN FAILED".to_string(), theme.danger),
    };

    let health = app.inventory.metrics.health;
    let mut right_spans = vec![Span::styled(
        status_text,
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD),
    )];
    right_spans.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
    right_spans.push(Span::styled(
        format!("HEALTH {health}"),
        theme.score_style(health),
    ));

    let right_line = Line::from(right_spans);
    let right_width = right_line.width() as u16;
    let left_width = area.width.saturating_sub(right_width).saturating_sub(1);

    let mut left_spans = vec![
        Span::styled(
            "▰",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " AMPH",
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
    ];

    if area.width >= 62 {
        left_spans.push(Span::styled(
            "  /  ARCH PACKAGE HARMONIZER",
            Style::default().fg(theme.muted),
        ));
    } else if area.width >= 46 {
        left_spans.push(Span::styled("  /  ARCH", Style::default().fg(theme.muted)));
    }

    let brand_area = Rect {
        x: area.x,
        y: area.y,
        width: left_width.max(1),
        height: 1,
    };
    frame.render_widget(Paragraph::new(Line::from(left_spans)), brand_area);

    let right_area = Rect {
        x: area.x + area.width.saturating_sub(right_width),
        y: area.y,
        width: right_width.min(area.width),
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(right_line).alignment(Alignment::Right),
        right_area,
    );

    if area.height > 1 {
        let subtitle_y = area.y + 1;
        let (left_text, left_color) = if app.stale_snapshot() {
            ("ARCH LINUX · SNAPSHOT STALE", theme.warning)
        } else if app.cleanup_enabled() {
            ("ARCH LINUX · CONTROLLED CLEANUP", theme.muted)
        } else {
            ("ARCH LINUX · READ-ONLY INTELLIGENCE", theme.muted)
        };
        let (right_text, right_color) = if app.scan.phase == ScanPhase::Failed {
            (
                format!(
                    "{} · {}",
                    if app.sample {
                        "SAMPLE PREVIEW"
                    } else {
                        "SCAN FAILED"
                    },
                    widgets::truncate(
                        app.scan.error.as_deref().unwrap_or("no snapshot received"),
                        area.width.saturating_sub(24).min(40)
                    )
                ),
                theme.danger,
            )
        } else if app.stale_snapshot() {
            (
                "PACKAGE SET STALE · RESTART TO RESCAN".to_string(),
                theme.warning,
            )
        } else if app.sample {
            (
                "ILLUSTRATIVE DATA · AWAITING SNAPSHOT".to_string(),
                theme.warning,
            )
        } else {
            (
                format!("SNAPSHOT · {}", app.inventory.scanned_at),
                theme.muted,
            )
        };

        frame.render_widget(
            Paragraph::new(Span::styled(left_text, Style::default().fg(left_color))),
            Rect {
                x: area.x,
                y: subtitle_y,
                width: area.width.saturating_sub(2),
                height: 1,
            },
        );

        let right_width = right_text.chars().count() as u16;
        if right_width + 2 < area.width {
            frame.render_widget(
                Paragraph::new(Span::styled(right_text, Style::default().fg(right_color)))
                    .alignment(Alignment::Right),
                Rect {
                    x: area.x + area.width.saturating_sub(right_width),
                    y: subtitle_y,
                    width: right_width,
                    height: 1,
                },
            );
        }
    }
}
