use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use crate::tui::app::{App, RowHit};
use crate::tui::metrics::{self, Check};
use crate::tui::theme::Theme;
use crate::tui::widgets;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    if area.height == 0 {
        return;
    }

    let theme = app.theme;
    let indices = app.check_indices();
    let filter = app.active_filter().to_string();
    let (pass, info, warn, fail) = metrics::check_counts(&app.inventory.checks);

    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let mut header_spans = vec![
        Span::styled(
            "CHECKS",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} item(s)", indices.len()),
            Style::default().fg(theme.muted),
        ),
        Span::styled("  ·  ", Style::default().fg(theme.border)),
    ];
    if area.width >= 64 {
        header_spans.extend([
            Span::styled(format!("{pass} PASS"), Style::default().fg(theme.positive)),
            Span::styled(format!("  {info} INFO"), Style::default().fg(theme.info)),
            Span::styled(format!("  {warn} WARN"), Style::default().fg(theme.warning)),
            Span::styled(format!("  {fail} FAIL"), Style::default().fg(theme.danger)),
        ]);
    } else {
        header_spans.extend([
            Span::styled(format!("{pass}P"), Style::default().fg(theme.positive)),
            Span::styled(format!(" {info}I"), Style::default().fg(theme.info)),
            Span::styled(format!(" {warn}W"), Style::default().fg(theme.warning)),
            Span::styled(format!(" {fail}F"), Style::default().fg(theme.danger)),
        ]);
    }
    if !filter.is_empty() {
        header_spans.push(Span::styled(
            format!("  ·  /{filter}"),
            Style::default().fg(theme.warning),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(header_spans)), chunks[0]);

    let table_area = chunks[1];
    if table_area.height < 3 {
        return;
    }

    let block = widgets::panel("Checklist", &theme);
    if indices.is_empty() {
        let inner = block.inner(table_area);
        frame.render_widget(block, table_area);
        let hint = if filter.is_empty() {
            "No checks were produced for this snapshot."
        } else {
            "Press Esc to clear the filter."
        };
        widgets::empty_state(frame, inner, "No checks match", hint, &theme);
        return;
    }

    let rows = indices
        .iter()
        .map(|index| check_row(&app.inventory.checks[*index], &theme))
        .collect::<Vec<_>>();

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Min(18),
            Constraint::Min(12),
        ],
    )
    .header(
        Row::new(vec!["STATUS", "CHECK", "SIGNAL"]).style(
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(block)
    .column_spacing(1)
    .highlight_symbol("▍ ")
    .row_highlight_style(theme.selection_style());

    let state = &mut app.checks_state;
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

fn check_row(check: &Check, theme: &Theme) -> Row<'static> {
    let color = theme.status_color(check.status);
    Row::new(vec![
        Cell::from(Span::styled(
            check.status.label(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Cell::from(Line::from(Span::styled(
            check.title.clone(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ))),
        Cell::from(Line::from(Span::styled(
            check.summary.clone(),
            Style::default().fg(theme.muted),
        ))),
    ])
}
