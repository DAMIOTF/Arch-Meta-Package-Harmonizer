use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Bar, BarChart, BarGroup, Gauge, Paragraph, Sparkline};

use crate::tui::app::App;
use crate::tui::model::{format_count, format_size};
use crate::tui::widgets;

fn compact_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.0} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.0} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    if area.height == 0 {
        return;
    }

    let columns = if area.width >= 88 { 4 } else { 2 };
    if area.height < 18 {
        render_kpis(frame, area, app, columns);
        return;
    }

    let kpi_height = if columns == 4 {
        5
    } else if area.height >= 24 {
        9
    } else {
        7
    };
    let middle_height = if area.height >= 20 { 5 } else { 4 };
    let chart_height = if area.height >= 20 { 6 } else { 5 };

    let sections = Layout::vertical([
        Constraint::Length(kpi_height),
        Constraint::Length(middle_height),
        Constraint::Length(chart_height),
        Constraint::Min(0),
    ])
    .spacing(1)
    .split(area);

    if sections[0].height > 0 {
        render_kpis(frame, sections[0], app, columns);
    }
    if sections[1].height > 0 {
        render_middle(frame, sections[1], app);
    }
    if sections[2].height > 0 {
        render_charts(frame, sections[2], app);
    }
    if sections[3].height > 2 {
        render_bottom(frame, sections[3], app);
    }
}

#[derive(Clone, Copy)]
enum Kpi {
    Packages,
    Footprint,
    Cleanup,
    Harmony,
}

fn render_kpis(frame: &mut Frame, area: Rect, app: &App, columns: usize) {
    if columns == 4 {
        let cells = Layout::horizontal([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .spacing(1)
        .split(area);
        for (index, kind) in [Kpi::Packages, Kpi::Footprint, Kpi::Cleanup, Kpi::Harmony]
            .into_iter()
            .enumerate()
        {
            render_kpi(frame, cells[index], app, kind);
        }
        return;
    }

    let row_height = (area.height.saturating_sub(1) / 2).clamp(3, 6);
    let rows = Layout::vertical([
        Constraint::Length(row_height),
        Constraint::Length(row_height),
    ])
    .spacing(1)
    .split(area);
    for (row_index, kinds) in [
        [Kpi::Packages, Kpi::Footprint],
        [Kpi::Cleanup, Kpi::Harmony],
    ]
    .into_iter()
    .enumerate()
    {
        let cells = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .spacing(1)
            .split(rows[row_index]);
        for (index, kind) in kinds.into_iter().enumerate() {
            render_kpi(frame, cells[index], app, kind);
        }
    }
}

fn render_kpi(frame: &mut Frame, area: Rect, app: &App, kind: Kpi) {
    if area.width < 6 || area.height < 3 {
        return;
    }
    let theme = &app.theme;
    let metrics = &app.inventory.metrics;

    let (title, value, subtitle, accent) = match kind {
        Kpi::Packages => (
            "Packages",
            if metrics.total_packages == 0 {
                "—".to_string()
            } else {
                format_count(metrics.total_packages)
            },
            format!("{} explicit · {} dep", metrics.explicit, metrics.dependency),
            theme.accent_alt,
        ),
        Kpi::Footprint => (
            "Footprint",
            if metrics.total_packages == 0 {
                "—".to_string()
            } else {
                format_size(metrics.total_size)
            },
            format!(
                "avg {} · {} heavy",
                compact_size(metrics.average_size),
                metrics.heavy_packages
            ),
            theme.info,
        ),
        Kpi::Cleanup => {
            let accent = if metrics.cleanup_candidates > 0 {
                theme.warning
            } else {
                theme.positive
            };
            (
                "Cleanup",
                if metrics.cleanup_candidates == 0 {
                    "clear".to_string()
                } else {
                    format_size(metrics.reclaimable)
                },
                format!("{} candidates", metrics.cleanup_candidates),
                accent,
            )
        }
        Kpi::Harmony => (
            "Harmony",
            if metrics.total_packages == 0 {
                "—".to_string()
            } else {
                format!("{}/100", metrics.harmony)
            },
            format!("{} health · {} foreign", metrics.health, metrics.foreign),
            theme.score_color(metrics.harmony),
        ),
    };

    let block = widgets::panel(title, theme).border_style(Style::default().fg(theme.border));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let value_line = Line::from(Span::styled(
        widgets::truncate(&value, inner.width),
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    ));
    let mut lines = vec![value_line];
    if inner.height > 1 {
        lines.push(Line::from(Span::styled(
            widgets::truncate(&subtitle, inner.width),
            Style::default().fg(theme.muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_middle(frame: &mut Frame, area: Rect, app: &App) {
    if area.width >= 44 {
        let columns = Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
            .spacing(1)
            .split(area);
        render_health(frame, columns[0], app);
        render_sparkline(frame, columns[1], app);
    } else {
        render_health(frame, area, app);
    }
}

fn render_health(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let metrics = &app.inventory.metrics;
    let block = widgets::panel("System health", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    let gauge = Gauge::default()
        .percent(metrics.health as u16)
        .label(Span::styled(
            format!(" {} / 100 ", metrics.health),
            Style::default()
                .fg(theme.text)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ))
        .gauge_style(
            Style::default()
                .fg(theme.score_color(metrics.health))
                .bg(theme.surface_alt)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(gauge, rows[0]);

    if rows[1].height > 0 {
        let attention = app
            .inventory
            .checks
            .iter()
            .filter(|check| check.status.is_attention())
            .count();
        let line = Line::from(vec![
            Span::styled(
                widgets::score_word(metrics.health),
                Style::default()
                    .fg(theme.score_color(metrics.health))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    " · {attention} signal{}",
                    if attention == 1 { "" } else { "s" }
                ),
                Style::default().fg(theme.muted),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), rows[1]);
    }
}

fn render_sparkline(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let metrics = &app.inventory.metrics;
    let block = widgets::panel("Health history", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    let stored = &app.inventory.health_history;
    let history = stored
        .iter()
        .rev()
        .take(40)
        .rev()
        .copied()
        .collect::<Vec<_>>();
    let data = if history.is_empty() {
        vec![metrics.health as u64]
    } else {
        history.iter().map(|value| u64::from(*value)).collect()
    };
    let sparkline = Sparkline::default()
        .data(data.iter().copied())
        .max(100)
        .style(
            Style::default()
                .fg(theme.score_color(metrics.health))
                .bg(theme.surface),
        );
    frame.render_widget(sparkline, rows[0]);

    if rows[1].height > 0 {
        let label = if history.is_empty() {
            format!("no stored history · current {}", metrics.health)
        } else {
            let latest = history.last().copied().unwrap_or(metrics.health);
            format!("{} samples · latest {latest}", history.len())
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                widgets::truncate(&label, inner.width),
                Style::default().fg(theme.muted),
            ))),
            rows[1],
        );
    }
}

fn render_charts(frame: &mut Frame, area: Rect, app: &App) {
    let columns = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .spacing(1)
        .split(area);
    render_bar_chart(frame, columns[0], app);
    render_treemap(frame, columns[1], app);
}

fn render_bar_chart(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let buckets = app.inventory.metrics.size_buckets;
    let data = [
        ("S", buckets[0].saturating_add(buckets[1]), theme.positive),
        ("M", buckets[2], theme.info),
        ("L", buckets[3], theme.warning),
        ("XL", buckets[4], theme.danger),
    ];
    let bars = data
        .into_iter()
        .map(|(label, value, color)| {
            Bar::default()
                .label(label)
                .value(value)
                .style(Style::default().fg(color))
        })
        .collect::<Vec<_>>();

    let chart = BarChart::default()
        .block(widgets::panel("Size buckets", theme))
        .data(BarGroup::new(bars))
        .bar_width(3)
        .bar_gap(1)
        .value_style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD))
        .label_style(Style::default().fg(theme.muted));
    frame.render_widget(chart, area);
}

fn render_treemap(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = widgets::panel("Disk treemap", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let palette = [
        theme.accent,
        theme.accent_alt,
        theme.info,
        theme.warning,
        theme.positive,
    ];
    let items = app
        .inventory
        .metrics
        .largest
        .iter()
        .take(5)
        .enumerate()
        .map(|(index, package)| {
            widgets::TreemapItem::new(
                package.name.clone(),
                package.size,
                palette[index % palette.len()],
            )
        })
        .collect::<Vec<_>>();

    if items.is_empty() {
        widgets::empty_state(
            frame,
            inner,
            "No disk data",
            "The snapshot contains no packages.",
            theme,
        );
        return;
    }

    frame.render_widget(widgets::DiskTreemap::new(&items, theme.bg), inner);
}

fn render_bottom(frame: &mut Frame, area: Rect, app: &App) {
    if area.width >= 76 {
        let columns = Layout::horizontal([Constraint::Percentage(56), Constraint::Percentage(44)])
            .spacing(1)
            .split(area);
        render_attention(frame, columns[0], app);
        render_heavy(frame, columns[1], app);
    } else {
        render_attention(frame, area, app);
    }
}

fn render_attention(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = widgets::panel("Attention", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut lines = Vec::new();
    for check in app
        .inventory
        .checks
        .iter()
        .filter(|check| check.status.is_attention())
        .take(inner.height as usize)
    {
        let color = theme.status_color(check.status);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", check.status.label()),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                widgets::truncate(&check.title, inner.width.saturating_sub(7)),
                Style::default().fg(theme.text),
            ),
        ]));
        if lines.len() < inner.height as usize {
            lines.push(Line::from(Span::styled(
                format!(
                    "  {}",
                    widgets::truncate(&check.summary, inner.width.saturating_sub(2))
                ),
                Style::default().fg(theme.muted),
            )));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "All checks are green.",
            Style::default()
                .fg(theme.positive)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "No orphan, dependency, or footprint alarms.",
            Style::default().fg(theme.muted),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_heavy(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = widgets::panel("Heavy hitters", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut lines = Vec::new();
    for package in app
        .inventory
        .metrics
        .largest
        .iter()
        .take(inner.height as usize)
    {
        let size = package.size_label();
        let name_width = inner.width.saturating_sub(size.chars().count() as u16);
        let name = widgets::truncate(&package.name, name_width.saturating_sub(1));
        let padding = name_width.saturating_sub(name.chars().count() as u16);
        lines.push(Line::from(vec![
            Span::styled(name, Style::default().fg(theme.text)),
            Span::raw(" ".repeat(padding as usize)),
            Span::styled(size, Style::default().fg(theme.warning)),
        ]));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No packages in this snapshot.",
            Style::default().fg(theme.muted),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}
