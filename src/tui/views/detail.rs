use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use crate::tui::app::{App, SessionEventKind, Tab, TimelineEntry};
use crate::tui::model::{Package, TimelineKind, format_size};
use crate::tui::theme::Theme;
use crate::tui::widgets;

pub fn render_side(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let block = widgets::panel("Inspect", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_content(frame, app, inner, false);
}

pub fn render_overlay(frame: &mut Frame, app: &App, screen: Rect) {
    let theme = &app.theme;
    let area = widgets::centered_rect(screen, 74, 84);
    frame.render_widget(Clear, area);
    let block = widgets::accent_panel("Inspect · esc close", theme, theme.accent);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_content(frame, app, inner, true);
}

fn render_content(frame: &mut Frame, app: &App, area: Rect, _overlay: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = &app.theme;
    let width = area.width;
    let mut lines = Vec::new();

    match app.active_tab {
        Tab::Packages | Tab::Graph | Tab::Cleanup => {
            if let Some(package) = app.selected_package() {
                package_lines(&mut lines, package, app, width, theme);
            } else {
                empty_lines(&mut lines, "No package selected", width, theme);
            }
        }
        Tab::Checks => {
            if let Some(check) = app.selected_check() {
                check_lines(&mut lines, check, width, theme);
            } else {
                empty_lines(&mut lines, "No check selected", width, theme);
            }
        }
        Tab::Timeline => {
            if let Some(entry) = app.selected_timeline() {
                timeline_entry_lines(&mut lines, &entry, app, width, theme);
            } else {
                empty_lines(&mut lines, "No timeline entry selected", width, theme);
            }
        }
        Tab::Dashboard => snapshot_lines(&mut lines, app, width, theme),
        Tab::Harmony => harmony_lines(&mut lines, app, width, theme),
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .style(theme.text_style())
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

fn package_lines(
    lines: &mut Vec<Line<'static>>,
    package: &Package,
    app: &App,
    width: u16,
    theme: &Theme,
) {
    lines.push(Line::from(Span::styled(
        package.name.clone(),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        widgets::truncate(&package.version, width),
        Style::default().fg(theme.muted),
    )));

    let mut badges = Vec::new();
    if package.is_orphan() {
        badges.push(badge("orphan", theme.warning, theme));
    } else if package.is_cleanup_candidate() {
        badges.push(badge("cleanup", theme.warning, theme));
    }
    if package.foreign {
        badges.push(badge("foreign", theme.accent_alt, theme));
    }
    if package.explicit {
        badges.push(badge("explicit", theme.info, theme));
    } else {
        badges.push(badge("dependency", theme.muted, theme));
    }
    if !badges.is_empty() {
        let mut spans = Vec::new();
        for (index, badge) in badges.into_iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(" "));
            }
            spans.push(badge);
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(vec![
        Span::styled(
            package.size_label(),
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" installed", Style::default().fg(theme.muted)),
    ]));

    if app.active_tab == Tab::Cleanup {
        let safety = if app.cleanup_enabled() {
            "Safety: external preview, then typed remove. The TUI runs no command."
        } else {
            "Read-only: no cleanup handler is attached; removal is disabled."
        };
        lines.push(Line::from(Span::styled(
            safety,
            Style::default().fg(theme.warning),
        )));
    }

    lines.push(separator(width, theme));
    if !package.description.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            package.description.clone(),
            Style::default().fg(theme.text),
        )));
        lines.push(Line::from(""));
    }

    if let Some(index) = app.selected_package_index() {
        let dependencies = app
            .inventory
            .graph
            .dependencies
            .get(index)
            .map(Vec::len)
            .unwrap_or(0);
        let dependents = app
            .inventory
            .graph
            .dependents
            .get(index)
            .map(Vec::len)
            .unwrap_or(0);
        lines.push(Line::from(vec![
            Span::styled("LINKS  ", Style::default().fg(theme.border)),
            Span::styled(
                format!("{dependencies} outgoing · {dependents} incoming"),
                Style::default().fg(theme.info),
            ),
        ]));
    }

    list_section(lines, "DEPENDS", &package.depends, 6, width, theme);
    list_section(lines, "PROVIDES", &package.provides, 4, width, theme);
    list_section(lines, "OPTIONAL", &package.optdepends, 5, width, theme);
}

fn check_lines(
    lines: &mut Vec<Line<'static>>,
    check: &crate::tui::metrics::Check,
    width: u16,
    theme: &Theme,
) {
    let color = theme.status_color(check.status);
    lines.push(Line::from(Span::styled(
        check.status.label(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        check.title.clone(),
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
    )));
    lines.push(separator(width, theme));
    lines.push(Line::from(Span::styled(
        check.summary.clone(),
        Style::default().fg(color),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        check.detail.clone(),
        Style::default().fg(theme.text),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "NEXT STEP",
        Style::default()
            .fg(theme.muted)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        check.action.clone(),
        Style::default().fg(theme.text),
    )));
}

fn timeline_entry_lines(
    lines: &mut Vec<Line<'static>>,
    entry: &TimelineEntry,
    app: &App,
    width: u16,
    theme: &Theme,
) {
    match entry {
        TimelineEntry::Transaction(item) => {
            let color = transaction_color(item.kind, theme);
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", item.kind.marker()),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    item.kind.label(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                item.timestamp.clone(),
                Style::default().fg(theme.muted),
            )));
            lines.push(Line::from(Span::styled(
                item.title.clone(),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            )));
            lines.push(separator(width, theme));
            lines.push(Line::from(Span::styled(
                if item.detail.is_empty() {
                    "No additional transaction detail.".to_string()
                } else {
                    item.detail.clone()
                },
                Style::default().fg(theme.text),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "SOURCE  pacman transaction history",
                Style::default().fg(theme.muted),
            )));
        }
        TimelineEntry::Session(event) => {
            let color = event_color(event.kind, theme);
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", event.kind.marker()),
                    Style::default().fg(color),
                ),
                Span::styled(
                    event.kind.label(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  T+{}", widgets::format_elapsed(event.at)),
                    Style::default().fg(theme.muted),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                event.title.clone(),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            )));
            lines.push(separator(width, theme));
            lines.push(Line::from(Span::styled(
                event.detail.clone(),
                Style::default().fg(theme.text),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("SNAPSHOT  ", Style::default().fg(theme.border)),
                Span::styled(
                    app.inventory.scanned_at.clone(),
                    Style::default().fg(theme.muted),
                ),
            ]));
            if app.sample {
                lines.push(Line::from(Span::styled(
                    "Preview data — no backend snapshot has arrived yet.",
                    Style::default().fg(theme.warning),
                )));
            }
        }
    }
}

fn snapshot_lines(lines: &mut Vec<Line<'static>>, app: &App, width: u16, theme: &Theme) {
    let metrics = &app.inventory.metrics;
    lines.push(Line::from(Span::styled(
        if app.sample {
            "SAMPLE SNAPSHOT"
        } else {
            "SNAPSHOT"
        },
        Style::default()
            .fg(if app.sample {
                theme.warning
            } else {
                theme.positive
            })
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        app.inventory.scanned_at.clone(),
        Style::default().fg(theme.muted),
    )));
    lines.push(separator(width, theme));
    lines.push(kv("PACKAGES", metrics.total_packages.to_string(), theme));
    lines.push(kv("FOOTPRINT", format_size(metrics.total_size), theme));
    lines.push(kv(
        "BALANCE",
        format!("{} explicit / {} dep", metrics.explicit, metrics.dependency),
        theme,
    ));
    lines.push(kv("HEALTH", format!("{}/100", metrics.health), theme));
    lines.push(kv("HARMONY", format!("{}/100", metrics.harmony), theme));
    lines.push(kv("FOREIGN", metrics.foreign.to_string(), theme));
    lines.push(kv("ORPHANS", metrics.orphans.to_string(), theme));
    lines.push(Line::from(""));
    if let Some(package) = metrics.largest.first() {
        lines.push(Line::from(Span::styled(
            "LARGEST PACKAGE",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled(
                widgets::truncate(&package.name, width.saturating_sub(10)),
                Style::default().fg(theme.text),
            ),
            Span::styled(
                format!("  {}", package.size_label()),
                Style::default().fg(theme.info),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        if app.cleanup_enabled() {
            "Cleanup uses the injected host handler; the TUI runs no command."
        } else {
            "Read-only interface. No cleanup handler is attached."
        },
        Style::default().fg(theme.muted),
    )));
    if app.stale_snapshot() {
        lines.push(Line::from(Span::styled(
            "The package set may be stale; restart to rescan.",
            Style::default().fg(theme.warning),
        )));
    }
}

fn harmony_lines(lines: &mut Vec<Line<'static>>, app: &App, width: u16, theme: &Theme) {
    let metrics = &app.inventory.metrics;
    lines.push(Line::from(Span::styled(
        "HARMONY INDEX",
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "{} / 100 · {}",
            metrics.harmony,
            widgets::score_word(metrics.harmony)
        ),
        Style::default().fg(theme.score_color(metrics.harmony)),
    )));
    lines.push(separator(width, theme));
    for (label, value) in [
        ("Structure", metrics.structure_score),
        ("Dependencies", metrics.dependency_score),
        ("Cleanliness", metrics.cleanliness_score),
        ("Provenance", metrics.provenance_score),
        ("Footprint", metrics.footprint_score),
    ] {
        lines.push(Line::from(vec![
            Span::styled(format!("{label:<13}"), Style::default().fg(theme.muted)),
            Span::styled(
                format!("{value:>3}"),
                Style::default()
                    .fg(theme.score_color(value))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Harmony is the average of structure, dependency integrity, cleanliness, provenance, and footprint.",
        Style::default().fg(theme.muted),
    )));
    lines.push(Line::from(""));
    if let Some(check) = app
        .inventory
        .checks
        .iter()
        .find(|check| check.status.is_attention())
    {
        lines.push(Line::from(Span::styled(
            check.title.clone(),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            check.summary.clone(),
            Style::default().fg(theme.text),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "No attention signals in this snapshot.",
            Style::default().fg(theme.positive),
        )));
    }
}

fn list_section(
    lines: &mut Vec<Line<'static>>,
    title: &str,
    items: &[String],
    limit: usize,
    width: u16,
    theme: &Theme,
) {
    lines.push(Line::from(vec![
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", items.len()),
            Style::default().fg(theme.border),
        ),
    ]));

    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            "  none",
            Style::default().fg(theme.muted),
        )));
        return;
    }

    for item in items.iter().take(limit) {
        lines.push(Line::from(vec![
            Span::styled("  · ", Style::default().fg(theme.border)),
            Span::styled(
                widgets::truncate(item, width.saturating_sub(4)),
                Style::default().fg(theme.text),
            ),
        ]));
    }
    if items.len() > limit {
        lines.push(Line::from(Span::styled(
            format!("  + {} more", items.len() - limit),
            Style::default().fg(theme.muted),
        )));
    }
}

fn separator(width: u16, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(1) as usize),
        Style::default().fg(theme.border),
    ))
}

fn kv(label: &str, value: String, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<11}"), Style::default().fg(theme.muted)),
        Span::styled(value, Style::default().fg(theme.text)),
    ])
}

fn badge(text: &str, color: Color, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {} ", text.to_ascii_uppercase()),
        theme.badge_style(color),
    )
}

fn empty_lines(lines: &mut Vec<Line<'static>>, title: &str, width: u16, theme: &Theme) {
    lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
    )));
    lines.push(separator(width, theme));
    lines.push(Line::from(Span::styled(
        "Move with j/k to reveal details.",
        Style::default().fg(theme.muted),
    )));
}

fn transaction_color(kind: TimelineKind, theme: &Theme) -> Color {
    match kind {
        TimelineKind::Install => theme.positive,
        TimelineKind::Upgrade => theme.info,
        TimelineKind::Remove => theme.danger,
        TimelineKind::Downgrade => theme.warning,
        TimelineKind::Reinstall => theme.accent_alt,
        TimelineKind::Other => theme.accent,
    }
}

fn event_color(kind: SessionEventKind, theme: &Theme) -> Color {
    match kind {
        SessionEventKind::Boot => theme.accent_alt,
        SessionEventKind::Scan => theme.info,
        SessionEventKind::Snapshot => theme.positive,
        SessionEventKind::Interface => theme.muted,
        SessionEventKind::Safety => theme.warning,
        SessionEventKind::Warning => theme.warning,
        SessionEventKind::Error => theme.danger,
    }
}
