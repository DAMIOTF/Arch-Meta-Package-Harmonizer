use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::tui::app::{App, RowHit, SessionEventKind, TimelineEntry};
use crate::tui::model::TimelineKind;
use crate::tui::theme::Theme;
use crate::tui::widgets;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    if area.height == 0 {
        return;
    }

    let theme = app.theme;
    let entries = app.filtered_timeline();
    let filter = app.active_filter().to_string();
    let pacman_count = app.inventory.timeline.len();
    let session_count = app.events.len();

    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let mut header_spans = vec![
        Span::styled(
            "TIMELINE",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  {} shown · {} pacman · {} session",
                entries.len(),
                pacman_count,
                session_count
            ),
            Style::default().fg(theme.muted),
        ),
    ];
    if !filter.is_empty() {
        header_spans.push(Span::styled(
            format!("  ·  /{filter}"),
            Style::default().fg(theme.warning),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(header_spans)), chunks[0]);

    let list_area = chunks[1];
    if list_area.height < 3 {
        return;
    }

    let block = widgets::panel("Activity", &theme);
    let inner = block.inner(list_area);
    frame.render_widget(block, list_area);

    if entries.is_empty() {
        widgets::empty_state(
            frame,
            inner,
            "No timeline entries match",
            "Press Esc to clear the filter.",
            &theme,
        );
        return;
    }

    let visible_items = (inner.height / 2) as usize;
    if visible_items == 0 {
        return;
    }
    let selected = app.timeline_selected.min(entries.len().saturating_sub(1));
    let offset = if selected >= visible_items {
        selected + 1 - visible_items
    } else {
        0
    };

    let mut hits = Vec::new();
    for row in 0..visible_items {
        let index = offset + row;
        if index >= entries.len() {
            break;
        }
        let entry = &entries[index];
        let y = inner.y + (row as u16) * 2;
        if y >= inner.y.saturating_add(inner.height) {
            break;
        }
        let item_height = 2.min(inner.y.saturating_add(inner.height).saturating_sub(y));
        let item_rect = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: item_height,
        };

        let is_selected = index == selected;
        if is_selected {
            frame.render_widget(Block::default().style(theme.selection_style()), item_rect);
        }

        let (time, color) = match entry {
            TimelineEntry::Transaction(item) => (
                widgets::truncate(&item.timestamp, 16),
                transaction_color(item.kind, &theme),
            ),
            TimelineEntry::Session(event) => (
                format!("T+{}", widgets::format_elapsed(event.at)),
                session_color(event.kind, &theme),
            ),
        };
        let first = Line::from(vec![
            Span::styled(
                if is_selected { "▍ " } else { "  " },
                Style::default().fg(theme.accent),
            ),
            Span::styled(format!("{time:<16}"), Style::default().fg(theme.muted)),
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{} ", entry.marker()),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<10}", entry.kind_label()),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                widgets::truncate(entry.title(), inner.width.saturating_sub(33)),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(first),
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
        );

        if item_height > 1 {
            let second = Line::from(vec![
                Span::styled("  ", Style::default().fg(theme.border)),
                Span::styled("│  ", Style::default().fg(theme.border)),
                Span::styled(
                    widgets::truncate(entry.detail(), inner.width.saturating_sub(6)),
                    Style::default().fg(theme.muted),
                ),
            ]);
            frame.render_widget(
                Paragraph::new(second),
                Rect {
                    x: inner.x,
                    y: y + 1,
                    width: inner.width,
                    height: 1,
                },
            );
        }

        hits.push(RowHit {
            rect: item_rect,
            index,
        });
    }

    widgets::render_scrollbar(
        frame,
        list_area,
        entries.len(),
        selected,
        visible_items,
        &theme,
    );
    app.hits.rows = hits;
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

fn session_color(kind: SessionEventKind, theme: &Theme) -> Color {
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
