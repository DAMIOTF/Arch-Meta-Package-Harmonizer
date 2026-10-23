use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::tui::app::{App, Overlay, Tab};
use crate::tui::widgets;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }

    let theme = &app.theme;
    frame.render_widget(Block::default().style(theme.mantle_style()), area);

    let (left_line, right_text) = if let Some(edit) = app.filter_edit() {
        let value = app.filters.get(edit.tab);
        let line = Line::from(vec![
            Span::styled(
                "FILTER",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" /", Style::default().fg(theme.border)),
            Span::styled(value.to_string(), Style::default().fg(theme.text)),
            Span::styled(
                "▌",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        (line, "ENTER APPLY · ESC CANCEL".to_string())
    } else {
        let hints = hints_for(app);
        (widgets::hint_line(&hints, theme), footer_right(app))
    };

    let mut right_text = right_text;
    if right_text.chars().count() as u16 + 12 >= area.width {
        right_text = app.theme.kind.label().to_string();
    }
    let right_width = right_text.chars().count() as u16;

    let columns = Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width.max(1))])
        .split(area);

    frame.render_widget(Paragraph::new(left_line), columns[0]);
    frame.render_widget(
        Paragraph::new(Span::styled(right_text, Style::default().fg(theme.muted)))
            .alignment(Alignment::Right),
        columns[1],
    );
}

fn hints_for(app: &App) -> Vec<(&'static str, &'static str)> {
    match &app.overlay {
        Overlay::Help => vec![("esc", "close"), ("?", "toggle")],
        Overlay::Detail => vec![("esc", "close"), ("i", "toggle panel")],
        Overlay::CleanupBusy => vec![("handler", "running")],
        Overlay::CleanupPreview(state) => {
            if state.can_remove {
                vec![
                    ("↑/↓", "scroll"),
                    ("type remove", "arm"),
                    ("enter", "run removal"),
                    ("esc", "cancel"),
                ]
            } else {
                vec![("↑/↓", "scroll"), ("esc", "close")]
            }
        }
        Overlay::CleanupResult(_) => vec![("↑/↓", "scroll"), ("esc", "close")],
        Overlay::CleanupError(_) => vec![("esc", "close")],
        Overlay::None => match app.active_tab {
            Tab::Dashboard => vec![
                ("1-7", "tabs"),
                ("/", "filter"),
                ("i", "inspect"),
                ("t", "theme"),
                ("?", "keys"),
                ("q", "quit"),
            ],
            Tab::Packages => vec![
                ("j/k", "move"),
                ("s", "sort"),
                ("/", "filter"),
                ("i", "inspect"),
                ("?", "keys"),
            ],
            Tab::Graph => vec![
                ("j/k", "node"),
                ("←/→", "branch"),
                ("space", "toggle"),
                ("/", "filter"),
                ("i", "detail"),
            ],
            Tab::Cleanup => vec![
                ("space", "pick"),
                ("a", "all"),
                ("p", "preview"),
                ("/", "filter"),
                ("?", "keys"),
            ],
            Tab::Harmony => vec![
                ("1-7", "tabs"),
                ("t", "theme"),
                ("?", "keys"),
                ("q", "quit"),
            ],
            Tab::Checks => vec![
                ("j/k", "move"),
                ("/", "filter"),
                ("i", "detail"),
                ("?", "keys"),
            ],
            Tab::Timeline => vec![
                ("j/k", "event"),
                ("/", "filter"),
                ("i", "detail"),
                ("?", "keys"),
            ],
        },
    }
}

fn footer_right(app: &App) -> String {
    let filter = app.active_filter();
    if !filter.is_empty() {
        return format!("/{}", widgets::truncate(filter, 18));
    }

    let mut label = app.theme.kind.label().to_string();
    if app.theme.is_monochrome() {
        label.push_str(" · NO_COLOR");
    }
    label
}
