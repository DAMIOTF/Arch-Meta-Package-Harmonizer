use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::tui::app::{App, PluginNoticeTone, PluginSection};
use crate::tui::plugins::PLUGIN_URL_MAX;
use crate::tui::theme::Theme;
use crate::tui::widgets;

/// The plugin manager is a workspace modal: the seven top-level tabs remain in
/// place and the current screen stays visible around the edges.
pub fn render_manager(frame: &mut Frame, screen: Rect, app: &App) {
    let theme = &app.theme;
    let stage = if app.layout.body.height >= 16 {
        app.layout.body
    } else {
        screen
    };
    let height = stage.height.saturating_sub(2).clamp(14, 36);
    let area = widgets::centered_rect_height(stage, 96, height);
    frame.render_widget(Clear, area);
    let block = widgets::accent_panel("Plugins · esc close", theme, theme.info);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height < 6 {
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);

    render_header(frame, rows[0], app);

    let content = rows[1];
    let body = if content.width >= 76 {
        let columns = Layout::horizontal([
            Constraint::Length(22),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(content);
        render_rail(frame, columns[0], app);
        frame.render_widget(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(Style::default().fg(theme.border)),
            columns[1],
        );
        columns[2]
    } else {
        let sections = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(content);
        render_section_tabs(frame, sections[0], app);
        sections[1]
    };

    render_body(frame, body, app);
    render_hints(frame, rows[2], app);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let theme = &app.theme;
    let state = &app.plugin_state;
    let lines = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);

    let summary = format!(
        "{} installed · {} update(s) ",
        state.rows.len(),
        state.update_count()
    );
    let summary_width = summary.chars().count() as u16;
    let show_summary = area.width >= 52 && summary_width.saturating_add(28) <= area.width;

    let mut title = vec![
        Span::styled(
            "PLUGIN MANAGER",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(theme.border)),
        Span::styled(
            state.section.label(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(theme.border)),
        Span::styled("WebAssembly sandbox", Style::default().fg(theme.muted)),
    ];
    if !app.plugins_enabled() {
        title.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
        title.push(Span::styled(
            "READ ONLY",
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ));
    }

    if show_summary {
        let columns = Layout::horizontal([Constraint::Min(0), Constraint::Length(summary_width)])
            .split(lines[0]);
        frame.render_widget(Paragraph::new(Line::from(title)), columns[0]);
        frame.render_widget(
            Paragraph::new(Span::styled(summary, Style::default().fg(theme.muted)))
                .alignment(Alignment::Right),
            columns[1],
        );
    } else {
        frame.render_widget(Paragraph::new(Line::from(title)), lines[0]);
    }

    let status = render_status_line(app);
    frame.render_widget(Paragraph::new(status), lines[1]);
}

fn render_status_line(app: &App) -> Line<'static> {
    let theme = &app.theme;
    let state = &app.plugin_state;

    if let Some(action) = &state.busy {
        return Line::from(vec![
            Span::styled(
                format!("{} ", widgets::spinner(app.tick)),
                Style::default().fg(theme.warning),
            ),
            Span::styled(
                action.label().to_ascii_uppercase(),
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  ·  handler running with the terminal suspended",
                Style::default().fg(theme.muted),
            ),
        ]);
    }

    if let Some(notice) = &state.notice {
        let (label, color) = notice_kind(notice.tone, theme);
        let mut spans = vec![
            Span::styled(label, theme.badge_style(color)),
            Span::raw(" "),
            Span::styled(
                notice.title.clone(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ];
        if let Some(detail) = &notice.detail {
            spans.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
            spans.push(Span::styled(
                detail.clone(),
                Style::default().fg(theme.muted),
            ));
        }
        return Line::from(spans);
    }

    if !app.plugins_enabled() {
        return Line::from(vec![
            Span::styled(
                "NO PLUGIN BACKEND ",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "review rows are visible; install, update, and remove are unavailable",
                Style::default().fg(theme.muted),
            ),
        ]);
    }

    Line::from(vec![
        Span::styled(
            "REVIEW FIRST ",
            Style::default()
                .fg(theme.positive)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "installs are reviewed · removal asks for confirmation",
            Style::default().fg(theme.muted),
        ),
    ])
}

fn notice_kind(tone: PluginNoticeTone, theme: &Theme) -> (&'static str, Color) {
    match tone {
        PluginNoticeTone::Info => (" NOTE ", theme.info),
        PluginNoticeTone::Success => (" DONE ", theme.positive),
        PluginNoticeTone::Warning => (" NOTICE ", theme.warning),
        PluginNoticeTone::Danger => (" ERROR ", theme.danger),
    }
}

fn render_rail(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = &app.theme;
    let state = &app.plugin_state;
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(PluginSection::ALL.len() as u16),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(area);

    frame.render_widget(
        Paragraph::new(Span::styled(
            "SECTIONS",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        )),
        rows[0],
    );

    for (index, section) in PluginSection::ALL.iter().enumerate() {
        let row = Rect {
            x: rows[1].x,
            y: rows[1].y.saturating_add(index as u16),
            width: rows[1].width,
            height: 1,
        };
        if row.y >= rows[1].y.saturating_add(rows[1].height) {
            break;
        }
        let selected = *section == state.section;
        if selected {
            frame.render_widget(
                Block::default().style(Style::default().bg(theme.surface_alt)),
                row,
            );
        }
        let count = match section {
            PluginSection::Installed => state.rows.len(),
            PluginSection::Add => 0,
            PluginSection::Updates => state.update_count(),
        };
        let marker = if selected { "› " } else { "  " };
        let label = format!("{}{} {}", marker, index + 1, section.short_label());
        let count_text = if matches!(section, PluginSection::Add) {
            String::new()
        } else {
            format!("{count:>3}")
        };
        let label_width = row
            .width
            .saturating_sub(count_text.chars().count() as u16 + 1);
        let mut spans = vec![Span::styled(
            format!(
                "{:<width$}",
                widgets::truncate(&label, label_width),
                width = label_width as usize
            ),
            if selected {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            },
        )];
        if !count_text.is_empty() {
            spans.push(Span::styled(
                count_text,
                Style::default().fg(if selected { theme.accent } else { theme.muted }),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), row);
    }

    let note = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(rows[4]);
    frame.render_widget(
        Paragraph::new(Span::styled("tab cycles", Style::default().fg(theme.muted))),
        note[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled("esc closes", Style::default().fg(theme.muted))),
        note[1],
    );
}

fn render_section_tabs(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = &app.theme;
    let state = &app.plugin_state;
    let mut spans = Vec::new();
    for (index, section) in PluginSection::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  ", Style::default().fg(theme.border)));
        }
        let selected = *section == state.section;
        let label = if area.width < 52 {
            section.short_label().to_string()
        } else {
            section.label().to_string()
        };
        spans.push(Span::styled(
            format!("{} {}", index + 1, label),
            if selected {
                theme.tab_active_style()
            } else {
                Style::default().fg(theme.muted)
            },
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_body(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let state = &app.plugin_state;
    if let Some(action) = &state.busy {
        render_busy(frame, area, app, action.label());
        return;
    }
    if let Some(confirm) = &state.analyze_confirm {
        render_analyze_confirm(frame, area, app, &confirm.name, confirm.package_count);
        return;
    }
    if let Some(confirm) = &state.remove_confirm {
        render_remove_confirm(frame, area, app, &confirm.name);
        return;
    }
    match state.section {
        PluginSection::Installed => render_installed(frame, area, app),
        PluginSection::Add => {
            if let Some(review) = &state.review {
                render_review(frame, area, app, &review.url, &review.repository);
            } else {
                render_add(frame, area, app);
            }
        }
        PluginSection::Updates => render_updates(frame, area, app),
    }
}

fn render_installed(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let state = &app.plugin_state;
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "INSTALLED PLUGINS",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {} total", state.rows.len()),
                Style::default().fg(theme.muted),
            ),
        ])),
        rows[0],
    );

    if state.rows.is_empty() {
        widgets::empty_state(
            frame,
            rows[2],
            "No plugins installed",
            "Press 2 or Tab to add one from a GitHub repository.",
            theme,
        );
        return;
    }

    let cols = row_columns(rows[2].width);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("  {:<name_width$}", "NAME", name_width = cols.0 as usize),
                Style::default().fg(theme.muted),
            ),
            Span::styled(
                format!(
                    "{:<version_width$}",
                    "VERSION",
                    version_width = cols.1 as usize
                ),
                Style::default().fg(theme.muted),
            ),
            Span::styled("SOURCE", Style::default().fg(theme.muted)),
        ])),
        rows[1],
    );

    let visible = rows[2].height as usize;
    let total = state.rows.len();
    let skip = if total <= visible {
        0
    } else {
        state
            .installed_cursor
            .saturating_sub(visible / 2)
            .min(total - visible)
    };
    for (offset, row) in state.rows.iter().skip(skip).take(visible).enumerate() {
        let index = skip + offset;
        let row_area = Rect {
            x: rows[2].x,
            y: rows[2].y.saturating_add(offset as u16),
            width: rows[2].width,
            height: 1,
        };
        let selected = index == state.installed_cursor;
        render_plugin_row(frame, row_area, row, selected, cols, theme);
    }

    let selected = state.selected_plugin();
    let mut detail = vec![Span::styled(
        selected
            .map(|row| row.display_name().to_string())
            .unwrap_or_else(|| "No selection".to_string()),
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
    )];
    if let Some(row) = selected {
        detail.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
        detail.push(Span::styled(
            row.version_label().to_string(),
            Style::default().fg(theme.muted),
        ));
        if !row.enabled() {
            detail.push(Span::styled(
                "  ·  DISABLED",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        detail.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
        detail.push(Span::styled(
            widgets::truncate(row.repository_label(), rows[3].width.saturating_sub(24)),
            Style::default().fg(theme.muted),
        ));
    }

    let mut facts = vec![Span::styled(
        selected
            .map(|row| row.kind().label().to_string())
            .unwrap_or_else(|| "plugin".to_string()),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(row) = selected {
        match row.kind() {
            crate::tui::plugins::PluginRowKind::Functionality => {
                facts.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
                facts.push(Span::styled(
                    row.api()
                        .map(|api| format!("ABI v{api}"))
                        .unwrap_or_else(|| "ABI not reported".to_string()),
                    Style::default().fg(theme.muted),
                ));
                if let Some(entry) = row.entry() {
                    facts.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
                    facts.push(Span::styled(
                        widgets::truncate(entry, 26),
                        Style::default().fg(theme.muted),
                    ));
                }
            }
            crate::tui::plugins::PluginRowKind::Theme => {
                facts.push(Span::styled(
                    "  ·  declarative",
                    Style::default().fg(theme.muted),
                ));
                if let Some(palette) = row.theme_palette() {
                    facts.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
                    facts.push(Span::styled(
                        palette.to_string(),
                        Style::default().fg(theme.muted),
                    ));
                }
            }
            crate::tui::plugins::PluginRowKind::Unknown => {}
        }
        if let Some(tag) = row.tag() {
            facts.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
            facts.push(Span::styled(
                widgets::truncate(tag, 20),
                Style::default().fg(theme.muted),
            ));
        }
        if let Some(description) = row.description() {
            facts.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
            facts.push(Span::styled(
                widgets::truncate(description, rows[3].width.saturating_sub(34)),
                Style::default().fg(theme.muted),
            ));
        }
        if !row.enabled() {
            facts.push(Span::styled(
                "  ·  enable to use",
                Style::default().fg(theme.warning),
            ));
        } else {
            match row.kind() {
                crate::tui::plugins::PluginRowKind::Functionality => {
                    facts.push(Span::styled(
                        "  ·  a analyze",
                        Style::default().fg(theme.accent),
                    ));
                }
                crate::tui::plugins::PluginRowKind::Theme => {
                    facts.push(Span::styled(
                        "  ·  T apply theme",
                        Style::default().fg(theme.accent),
                    ));
                }
                crate::tui::plugins::PluginRowKind::Unknown => {}
            }
        }
        facts.push(Span::styled(
            "  ·  space on/off",
            Style::default().fg(theme.accent),
        ));
    }

    frame.render_widget(
        Paragraph::new(Text::from(vec![Line::from(detail), Line::from(facts)]))
            .wrap(Wrap { trim: true }),
        rows[3],
    );
}

fn row_columns(width: u16) -> (u16, u16) {
    let version = if width >= 64 { 13 } else { 10 };
    let name = ((width.saturating_sub(version).saturating_sub(14) * 45) / 100).clamp(10, 32);
    (name, version)
}

fn render_plugin_row(
    frame: &mut Frame,
    area: Rect,
    row: &crate::tui::plugins::PluginRow,
    selected: bool,
    columns: (u16, u16),
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if selected {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.surface_alt)),
            area,
        );
    }
    let name_text = if row.has_update() {
        format!("{} ▲", row.display_name())
    } else {
        row.display_name().to_string()
    };
    let name = format!(
        " {}{:<width$}",
        if selected { "›" } else { " " },
        widgets::truncate(&name_text, columns.0.saturating_sub(2)),
        width = columns.0.saturating_sub(2) as usize
    );
    let version = format!(
        "{:<width$}",
        widgets::truncate(row.version_label(), columns.1),
        width = columns.1 as usize
    );
    let source_width = area
        .width
        .saturating_sub(columns.0)
        .saturating_sub(columns.1)
        .saturating_sub(1);
    let source = widgets::truncate(row.repository_label(), source_width);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                name,
                Style::default()
                    .fg(if selected {
                        theme.accent
                    } else if row.enabled() {
                        theme.text
                    } else {
                        theme.muted
                    })
                    .add_modifier(if selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(version, Style::default().fg(theme.muted)),
            Span::styled(source, Style::default().fg(theme.muted)),
        ])),
        area,
    );
}

fn render_updates(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let state = &app.plugin_state;
    let updates = state.update_rows();
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "PLUGIN UPDATES",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {} available", updates.len()),
                Style::default().fg(if updates.is_empty() {
                    theme.muted
                } else {
                    theme.warning
                }),
            ),
            Span::styled("  ·  c check", Style::default().fg(theme.muted)),
        ])),
        rows[0],
    );

    if updates.is_empty() {
        let (title, hint) = if state.updates_checked {
            ("No updates were reported", "Press c to check again.")
        } else {
            (
                "No update check yet",
                "Press c to ask the host for available plugin updates.",
            )
        };
        widgets::empty_state(frame, rows[2], title, hint, theme);
        return;
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  NAME", Style::default().fg(theme.muted)),
            Span::styled("  CURRENT → LATEST", Style::default().fg(theme.muted)),
        ])),
        rows[1],
    );

    let visible = rows[2].height as usize;
    let total = updates.len();
    let skip = if total <= visible {
        0
    } else {
        state
            .update_cursor
            .saturating_sub(visible / 2)
            .min(total - visible)
    };
    for (offset, row) in updates.iter().skip(skip).take(visible).enumerate() {
        let index = skip + offset;
        let row_area = Rect {
            x: rows[2].x,
            y: rows[2].y.saturating_add(offset as u16),
            width: rows[2].width,
            height: 1,
        };
        let selected = index == state.update_cursor;
        if selected {
            frame.render_widget(
                Block::default().style(Style::default().bg(theme.surface_alt)),
                row_area,
            );
        }
        let latest = row.latest_version().unwrap_or("unknown");
        let line = Line::from(vec![
            Span::styled(
                format!(
                    " {} {:<name_width$}",
                    if selected { "›" } else { " " },
                    widgets::truncate(
                        row.display_name(),
                        row_area.width.saturating_sub(28).clamp(8, 28)
                    ),
                    name_width = row_area.width.saturating_sub(28).clamp(8, 28) as usize
                ),
                Style::default()
                    .fg(if selected { theme.accent } else { theme.text })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} → ", row.version_label()),
                Style::default().fg(theme.muted),
            ),
            Span::styled(
                widgets::truncate(latest, 18),
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), row_area);
    }

    let selected = updates.get(state.update_cursor).copied();
    let mut detail = vec![Span::styled(
        selected
            .map(|row| row.display_name().to_string())
            .unwrap_or_else(|| "No selection".to_string()),
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
    )];
    if let Some(row) = selected {
        detail.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
        detail.push(Span::styled(
            widgets::truncate(row.repository_label(), rows[3].width.saturating_sub(24)),
            Style::default().fg(theme.muted),
        ));
        detail.push(Span::styled(
            "  ·  U update all",
            Style::default().fg(theme.accent),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(detail)).wrap(Wrap { trim: true }),
        rows[3],
    );
}

fn render_add(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let state = &app.plugin_state;
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "ADD FROM GITHUB",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  ·  normalized before review",
                Style::default().fg(theme.muted),
            ),
        ])),
        rows[0],
    );

    frame.render_widget(
        Paragraph::new(
            "Paste a GitHub repository URL. AMPH shows a review before the host \
             installs anything. Use Ctrl+Shift+V or your terminal's paste shortcut.",
        )
        .style(Style::default().fg(theme.muted))
        .wrap(Wrap { trim: true }),
        rows[1],
    );

    let input_block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .style(theme.panel_style())
        .title(Line::from(Span::styled(
            " GitHub repository ",
            Style::default().fg(theme.muted),
        )));
    let input_inner = input_block.inner(rows[2]);
    frame.render_widget(input_block, rows[2]);

    if input_inner.width > 0 && input_inner.height > 0 {
        let (value, style) = if state.url_input.is_empty() {
            (
                "https://github.com/owner/repo".to_string(),
                Style::default().fg(theme.muted),
            )
        } else {
            (
                widgets::truncate(&state.url_input, input_inner.width.saturating_sub(1)),
                Style::default().fg(theme.text),
            )
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(value, style),
                Span::styled(
                    "▌",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            input_inner,
        );
    }

    if let Some(error) = &state.url_error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                widgets::truncate(error, rows[3].width),
                Style::default().fg(theme.danger),
            )),
            rows[3],
        );
    } else {
        let counter = format!(
            "{} / {PLUGIN_URL_MAX} characters",
            state.url_input.chars().count()
        );
        frame.render_widget(
            Paragraph::new(Span::styled(counter, Style::default().fg(theme.muted)))
                .alignment(Alignment::Right),
            rows[3],
        );
    }

    if !app.plugins_enabled() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No plugin backend is attached: review stays available, install does not.",
                Style::default().fg(theme.warning),
            )),
            rows[4],
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "ENTER",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" review install", Style::default().fg(theme.muted)),
            Span::styled("  ·  ", Style::default().fg(theme.border)),
            Span::styled("TAB", Style::default().fg(theme.accent)),
            Span::styled(" sections", Style::default().fg(theme.muted)),
            Span::styled("  ·  ", Style::default().fg(theme.border)),
            Span::styled("ESC", Style::default().fg(theme.accent)),
            Span::styled(" close", Style::default().fg(theme.muted)),
        ]))
        .wrap(Wrap { trim: true }),
        rows[6],
    );
}

fn render_review(frame: &mut Frame, area: Rect, app: &App, url: &str, repository: &str) {
    let theme = &app.theme;
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "REVIEW INSTALL",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  ·  nothing has been installed",
                Style::default().fg(theme.muted),
            ),
        ])),
        rows[0],
    );

    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                Span::styled("REPOSITORY  ", Style::default().fg(theme.muted)),
                Span::styled(
                    widgets::truncate(repository, rows[1].width.saturating_sub(14)),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("URL         ", Style::default().fg(theme.muted)),
                Span::styled(
                    widgets::truncate(url, rows[1].width.saturating_sub(14)),
                    Style::default().fg(theme.muted),
                ),
            ]),
        ])),
        rows[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("SANDBOX  ", Style::default().fg(theme.muted)),
            Span::styled(
                "WebAssembly",
                Style::default()
                    .fg(theme.positive)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[2],
    );

    frame.render_widget(
        Paragraph::new(
            "The host installs from the latest GitHub Release. Both kinds ship \
             plugin.json; a theme plugin is declarative data, while a functionality \
             plugin adds a Wasm entry using ABI v1 and runs in the sandbox. Every \
             release asset must carry a GitHub SHA-256 digest, verified before \
             install. Only install code you trust.",
        )
        .style(Style::default().fg(theme.muted))
        .wrap(Wrap { trim: true }),
        rows[3],
    );

    let action = if app.plugins_enabled() {
        Line::from(vec![
            Span::styled(" ENTER", button_style(theme, theme.positive)),
            Span::styled(
                " install through the host",
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled("   ·   ESC back", Style::default().fg(theme.muted)),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                " INSTALL UNAVAILABLE ",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "no plugin backend is attached to this session",
                Style::default().fg(theme.muted),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(action).wrap(Wrap { trim: true }), rows[5]);

    frame.render_widget(
        Paragraph::new(Span::styled(
            "Esc returns to the URL without installing. Enter sends the action to the host.",
            Style::default().fg(theme.muted),
        ))
        .wrap(Wrap { trim: true }),
        rows[6],
    );
}

fn render_busy(frame: &mut Frame, area: Rect, app: &App, label: &str) {
    let theme = &app.theme;
    let lines = vec![
        Line::from(Span::styled(
            format!(
                "{} {}",
                widgets::spinner(app.tick),
                label.to_ascii_uppercase()
            ),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "The host is working. The terminal may be suspended until it returns.",
            Style::default().fg(theme.muted),
        )),
        Line::from(Span::styled(
            "The manager refreshes when the result arrives.",
            Style::default().fg(theme.muted),
        )),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_analyze_confirm(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    name: &str,
    package_count: usize,
) {
    let theme = &app.theme;
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(area);

    frame.render_widget(
        Paragraph::new(Span::styled(
            "ANALYZE PACKAGES",
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        )),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                Span::styled("Plugin   ", Style::default().fg(theme.muted)),
                Span::styled(
                    widgets::truncate(name, rows[1].width.saturating_sub(11)),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Snapshot ", Style::default().fg(theme.muted)),
                Span::styled(
                    format!("{package_count} package(s)"),
                    Style::default().fg(theme.text),
                ),
            ]),
        ])),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(
            "The current package snapshot is sent to the plugin inside the host \
             sandbox. No host handles, paths, or capabilities are included.",
        )
        .style(Style::default().fg(theme.muted))
        .wrap(Wrap { trim: true }),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ENTER", button_style(theme, theme.info)),
            Span::styled(
                " analyze",
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled("   ·   ESC cancel", Style::default().fg(theme.muted)),
        ]))
        .wrap(Wrap { trim: true }),
        rows[4],
    );
}

fn render_remove_confirm(frame: &mut Frame, area: Rect, app: &App, name: &str) {
    let theme = &app.theme;
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(area);

    frame.render_widget(
        Paragraph::new(Span::styled(
            "REMOVE PLUGIN",
            Style::default()
                .fg(theme.danger)
                .add_modifier(Modifier::BOLD),
        )),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Plugin  ", Style::default().fg(theme.muted)),
            Span::styled(
                widgets::truncate(name, rows[1].width.saturating_sub(10)),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(
            "This asks the host to remove the plugin. Nothing has been touched yet; \
             press Esc to keep it.",
        )
        .style(Style::default().fg(theme.muted))
        .wrap(Wrap { trim: true }),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ENTER", button_style(theme, theme.danger)),
            Span::styled(
                " remove",
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled("   ·   ESC cancel", Style::default().fg(theme.muted)),
        ]))
        .wrap(Wrap { trim: true }),
        rows[4],
    );
}

fn render_hints(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = &app.theme;
    let hints: Vec<(&str, &str)> = if let Some(action) = &app.plugin_state.busy {
        vec![("handler", action.label())]
    } else if app.plugin_state.analyze_confirm.is_some() {
        vec![("enter", "analyze"), ("esc", "cancel")]
    } else if app.plugin_state.remove_confirm.is_some() {
        vec![("enter", "remove"), ("esc", "cancel")]
    } else if app.plugin_state.review.is_some() {
        vec![("enter", "install"), ("esc", "back")]
    } else {
        match app.plugin_state.section {
            PluginSection::Installed => {
                let mut hints = vec![("j/k", "move"), ("space", "on/off")];
                if let Some(row) = app.plugin_state.selected_plugin()
                    && row.enabled()
                {
                    match row.kind() {
                        crate::tui::plugins::PluginRowKind::Functionality => {
                            hints.push(("a", "analyze"));
                        }
                        crate::tui::plugins::PluginRowKind::Theme => {
                            hints.push(("T", "apply theme"));
                        }
                        crate::tui::plugins::PluginRowKind::Unknown => {}
                    }
                }
                hints.extend([
                    ("r", "remove"),
                    ("c", "updates"),
                    ("U", "update all"),
                    ("tab", "sections"),
                    ("esc", "close"),
                ]);
                hints
            }
            PluginSection::Add => vec![
                ("type / paste", "URL"),
                ("enter", "review"),
                ("tab", "sections"),
                ("esc", "close"),
            ],
            PluginSection::Updates if app.plugin_update_count() == 0 => {
                vec![("enter", "check"), ("c", "check"), ("esc", "close")]
            }
            PluginSection::Updates => vec![
                ("j/k", "move"),
                ("enter", "update all"),
                ("U", "update all"),
                ("c", "check"),
                ("esc", "close"),
            ],
        }
    };
    frame.render_widget(
        Paragraph::new(widgets::hint_line(&hints, theme)).wrap(Wrap { trim: true }),
        area,
    );
}

fn button_style(theme: &Theme, color: Color) -> Style {
    if theme.is_monochrome() {
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.bg)
            .bg(color)
            .add_modifier(Modifier::BOLD)
    }
}
