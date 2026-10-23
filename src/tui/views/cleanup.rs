use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use crate::tui::app::{App, RowHit};
use crate::tui::model::{Package, format_size};
use crate::tui::theme::Theme;
use crate::tui::widgets;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    if area.height == 0 {
        return;
    }

    let theme = app.theme;
    let indices = app.cleanup_indices();
    let filter = app.active_filter().to_string();
    let selected_count = app.cleanup_selected_count();
    let selected_bytes = app.cleanup_selected_bytes();

    let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(area);
    let mut first_spans = vec![
        Span::styled(
            "CLEANUP",
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  {} candidate{} · {} selected · {}",
                indices.len(),
                if indices.len() == 1 { "" } else { "s" },
                selected_count,
                format_size(selected_bytes)
            ),
            Style::default().fg(theme.muted),
        ),
    ];
    if !filter.is_empty() {
        first_spans.push(Span::styled(
            format!("  ·  /{filter}"),
            Style::default().fg(theme.warning),
        ));
    }
    if !indices.is_empty() && app.cleanup_all_visible_selected() {
        first_spans.push(Span::styled(
            "  ·  ALL VISIBLE",
            Style::default()
                .fg(theme.positive)
                .add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(first_spans)), chunks[0]);

    let safety = if app.cleanup_enabled() {
        "External preview · typed confirmation required"
    } else {
        "Read-only session · no handler attached"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("⚠ ", Style::default().fg(theme.warning)),
            Span::styled(safety, Style::default().fg(theme.muted)),
        ])),
        Rect {
            x: chunks[0].x,
            y: chunks[0].y + 1,
            width: chunks[0].width,
            height: 1,
        },
    );

    let table_area = chunks[1];
    if table_area.height < 3 {
        return;
    }

    let block = widgets::panel("Cleanup queue", &theme);
    if indices.is_empty() {
        let inner = block.inner(table_area);
        frame.render_widget(block, table_area);
        let hint = if filter.is_empty() {
            "Nothing is flagged as an orphan or unused dependency."
        } else {
            "Press Esc to clear the filter."
        };
        widgets::empty_state(frame, inner, "Cleanup queue is clear", hint, &theme);
        return;
    }

    let rows = indices
        .iter()
        .map(|index| {
            cleanup_row(
                &app.inventory.packages[*index],
                app.cleanup_selected
                    .contains(&app.inventory.packages[*index].name),
                &theme,
            )
        })
        .collect::<Vec<_>>();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Min(16),
            Constraint::Length(10),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec!["SEL", "PACKAGE", "SIZE", "REASON"]).style(
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(block)
    .column_spacing(1)
    .highlight_symbol("▍ ")
    .row_highlight_style(theme.selection_style());

    let state = &mut app.cleanup_state;
    frame.render_stateful_widget(table, table_area, state);

    let offset = state.offset();
    let selected = state.selected().unwrap_or(0);
    let visible_rows = table_area.height.saturating_sub(3);
    widgets::render_scrollbar(
        frame,
        table_area,
        indices.len(),
        selected,
        visible_rows as usize,
        &theme,
    );

    let mut hits = Vec::new();
    for row in 0..visible_rows {
        let index = offset + row as usize;
        if index >= indices.len() {
            break;
        }
        hits.push(RowHit {
            rect: Rect {
                x: table_area.x.saturating_add(1),
                y: table_area.y.saturating_add(2).saturating_add(row),
                width: table_area.width.saturating_sub(2),
                height: 1,
            },
            index,
        });
    }
    app.hits.rows = hits;
}

fn cleanup_row(package: &Package, selected: bool, theme: &Theme) -> Row<'static> {
    let marker = if selected {
        Span::styled(
            "[x]",
            Style::default()
                .fg(theme.positive)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("[ ]", Style::default().fg(theme.muted))
    };

    Row::new(vec![
        Cell::from(Line::from(marker)),
        Cell::from(Line::from(Span::styled(
            package.name.clone(),
            Style::default()
                .fg(if selected { theme.accent } else { theme.text })
                .add_modifier(Modifier::BOLD),
        ))),
        Cell::from(
            Line::from(Span::styled(
                package.size_label(),
                Style::default().fg(theme.warning),
            ))
            .alignment(Alignment::Right),
        ),
        Cell::from(Span::styled(
            widgets::truncate(package.reason_label(), 14),
            Style::default().fg(theme.muted),
        )),
    ])
}
