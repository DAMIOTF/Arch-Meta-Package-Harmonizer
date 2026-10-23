use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use crate::tui::app::{App, RowHit};
use crate::tui::model::Package;
use crate::tui::theme::Theme;
use crate::tui::widgets;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    if area.height == 0 {
        return;
    }

    let theme = app.theme;
    let indices = app.package_indices();
    let total = app.inventory.packages.len();
    let filter = app.active_filter().to_string();

    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let toolbar = toolbar_line(app, indices.len(), total, &filter, area.width);
    frame.render_widget(Paragraph::new(toolbar), chunks[0]);

    let table_area = chunks[1];
    if table_area.height < 3 {
        return;
    }

    let block = widgets::panel("Inventory", &theme);
    if indices.is_empty() {
        let inner = block.inner(table_area);
        frame.render_widget(block, table_area);
        let hint = if filter.is_empty() {
            "No packages are available in this snapshot."
        } else {
            "Press Esc to clear the filter."
        };
        widgets::empty_state(frame, inner, "Nothing to show", hint, &theme);
        return;
    }

    let widths = column_widths(table_area.width);
    let header = header_row(table_area.width, &theme);
    let rows = indices
        .iter()
        .map(|index| package_row(&app.inventory.packages[*index], table_area.width, &theme))
        .collect::<Vec<_>>();

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .highlight_symbol("▍ ")
        .row_highlight_style(theme.selection_style());

    let state = &mut app.package_state;
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

fn toolbar_line(
    app: &App,
    visible: usize,
    total: usize,
    filter: &str,
    width: u16,
) -> Line<'static> {
    let theme = &app.theme;
    let sort = app.package_sort.label();
    let orphans = app.inventory.metrics.orphans;
    let foreign = app.inventory.metrics.foreign;
    let mut spans = vec![
        Span::styled(
            "PACKAGES",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {visible}/{total} shown"),
            Style::default().fg(theme.muted),
        ),
        Span::styled("  ·  ", Style::default().fg(theme.border)),
        Span::styled(format!("SORT {sort}"), Style::default().fg(theme.info)),
    ];

    if !filter.is_empty() {
        spans.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
        spans.push(Span::styled(
            format!("/{filter}"),
            Style::default().fg(theme.warning),
        ));
    } else if width >= 64 {
        spans.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
        spans.push(Span::styled(
            format!("{orphans} orphan · {foreign} foreign"),
            Style::default().fg(theme.muted),
        ));
    }

    Line::from(spans)
}

fn column_widths(width: u16) -> Vec<Constraint> {
    if width >= 88 {
        vec![
            Constraint::Min(24),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Length(9),
        ]
    } else if width >= 64 {
        vec![
            Constraint::Min(18),
            Constraint::Length(10),
            Constraint::Length(11),
            Constraint::Length(8),
        ]
    } else {
        vec![
            Constraint::Min(16),
            Constraint::Length(10),
            Constraint::Length(10),
        ]
    }
}

fn header_row(width: u16, theme: &Theme) -> Row<'static> {
    let headers = if width >= 88 {
        vec!["NAME", "VERSION", "SIZE", "REASON", "FLAGS"]
    } else if width >= 64 {
        vec!["NAME", "SIZE", "REASON", "FLAGS"]
    } else {
        vec!["NAME", "SIZE", "REASON"]
    };

    Row::new(headers)
        .style(
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(0)
}

fn package_row(package: &Package, width: u16, theme: &Theme) -> Row<'static> {
    let name_color = if package.is_orphan() {
        theme.warning
    } else if package.foreign {
        theme.accent
    } else {
        theme.text
    };

    let name = Cell::from(Line::from(vec![
        Span::styled(
            package.name.clone(),
            Style::default().fg(name_color).add_modifier(Modifier::BOLD),
        ),
        if package.foreign {
            Span::styled(" ◇", Style::default().fg(theme.muted))
        } else {
            Span::raw("")
        },
    ]));

    let size = Cell::from(
        Line::from(Span::styled(
            package.size_label(),
            Style::default().fg(theme.info),
        ))
        .alignment(Alignment::Right),
    );

    let reason_color = if package.is_cleanup_candidate() {
        theme.warning
    } else if package.explicit {
        theme.text
    } else {
        theme.muted
    };
    let reason = Cell::from(Line::from(Span::styled(
        widgets::truncate(package.reason_label(), 14),
        Style::default().fg(reason_color),
    )));

    if width >= 88 {
        Row::new(vec![
            name,
            Cell::from(Span::styled(
                widgets::truncate(&package.version, 12),
                Style::default().fg(theme.muted),
            )),
            size,
            reason,
            flags_cell(package, theme),
        ])
    } else if width >= 64 {
        Row::new(vec![name, size, reason, flags_cell(package, theme)])
    } else {
        Row::new(vec![name, size, reason])
    }
}

fn flags_cell(package: &Package, theme: &Theme) -> Cell<'static> {
    let mut flags = String::new();
    if package.is_orphan() {
        flags.push('O');
    } else if package.is_cleanup_candidate() {
        flags.push('C');
    }
    if package.foreign {
        flags.push('F');
    }
    if flags.is_empty() {
        flags.push('·');
    }

    let color = if package.is_orphan() || package.is_cleanup_candidate() {
        theme.warning
    } else if package.foreign {
        theme.accent
    } else {
        theme.muted
    };

    Cell::from(Span::styled(flags, Style::default().fg(color)))
}
