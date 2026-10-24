use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::tui::app::{App, UpdateResultState};
use crate::tui::theme::Theme;
use crate::tui::update::UPDATE_CHANNELS;
use crate::tui::widgets;

/// One-line, non-blocking notice under the tab bar.
///
/// It stays for the rest of the session: first as an install invitation, then
/// as a restart reminder. It is never a modal, so the screen in use stays
/// visible and keyboard focus does not move.
pub fn banner(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.height == 0 || app.update_notice().is_none() {
        return;
    }
    app.hits.update = Some(area);

    let theme = app.theme;
    let notice = app.update_notice().expect("checked above");
    let installed = notice.installed();
    let accent = if installed {
        theme.positive
    } else {
        theme.warning
    };
    let latest = notice.info().latest().to_string();
    let background = Style::default().fg(theme.text).bg(theme.surface_alt);

    frame.render_widget(Block::default().style(background), area);

    if area.width < 56 {
        let text = if installed {
            format!(" ▲ RESTART AMPH · {latest} ")
        } else if app.update_enabled() {
            format!(" ▲ UPDATE · u install · {latest} ")
        } else {
            format!(" ▲ UPDATE · {latest} ")
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                text,
                background.fg(accent).add_modifier(Modifier::BOLD),
            )),
            area,
        );
        return;
    }

    let mut left = vec![
        Span::styled(
            " ▲ ",
            Style::default()
                .fg(accent)
                .bg(theme.surface_alt)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if installed {
                "RESTART AMPH"
            } else {
                "UPDATE AVAILABLE"
            },
            Style::default()
                .fg(accent)
                .bg(theme.surface_alt)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", background),
    ];

    if installed {
        left.push(Span::styled(
            format!("{latest} is installed · restart to load it"),
            Style::default().fg(theme.text).bg(theme.surface_alt),
        ));
    } else {
        left.push(Span::styled(
            notice.info().current().to_string(),
            Style::default().fg(theme.muted).bg(theme.surface_alt),
        ));
        left.push(Span::styled(
            " → ",
            Style::default().fg(theme.border).bg(theme.surface_alt),
        ));
        left.push(Span::styled(
            latest.clone(),
            Style::default()
                .fg(theme.text)
                .bg(theme.surface_alt)
                .add_modifier(Modifier::BOLD),
        ));
        left.push(Span::styled(
            "  ·  ",
            Style::default().fg(theme.border).bg(theme.surface_alt),
        ));
        if app.update_enabled() {
            left.push(Span::styled(
                "u",
                Style::default()
                    .fg(theme.accent)
                    .bg(theme.surface_alt)
                    .add_modifier(Modifier::BOLD),
            ));
            left.push(Span::styled(
                " install",
                Style::default().fg(theme.muted).bg(theme.surface_alt),
            ));
        } else {
            left.push(Span::styled(
                "installer not attached",
                Style::default().fg(theme.muted).bg(theme.surface_alt),
            ));
        }
    }

    let line = Line::from(left);
    let left_width = line.width() as u16;
    let right = format!("CHANNEL {} ", notice.info().branch().to_ascii_uppercase());
    let right_width = right.chars().count() as u16;
    frame.render_widget(Paragraph::new(line), area);

    if left_width.saturating_add(right_width).saturating_add(2) <= area.width {
        frame.render_widget(
            Paragraph::new(Span::styled(
                right,
                Style::default()
                    .fg(theme.muted)
                    .bg(theme.surface_alt)
                    .add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Right),
            area,
        );
    }
}

/// Modal channel picker opened with `Shift+V`.
///
/// The selected channel is marked with a filled dot and a `current` tag. The
/// action line states exactly what Enter will do, including the safe no-op when
/// the highlighted channel is already selected.
pub fn render_channel_picker(frame: &mut Frame, screen: Rect, app: &App) {
    let theme = &app.theme;
    let stage = stage(app, screen);
    let area = widgets::centered_rect_height(stage, 72, 13);
    frame.render_widget(Clear, area);
    let block = widgets::accent_panel("Update channel · shift+v close", theme, theme.info);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let content = inner.inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    if content.width == 0 || content.height == 0 {
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(UPDATE_CHANNELS.len() as u16),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(content);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "SELECT A CHANNEL",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  ", Style::default().fg(theme.border)),
            Span::styled("installs immediately", Style::default().fg(theme.muted)),
        ])),
        rows[0],
    );

    frame.render_widget(
        Paragraph::new(
            "Switching fetches the latest build from that branch and installs it. \
             The host saves the channel; restart AMPH afterwards to load the new build.",
        )
        .style(Style::default().fg(theme.muted))
        .wrap(Wrap { trim: true }),
        rows[1],
    );

    let list = Rect {
        height: rows[3].height.min(UPDATE_CHANNELS.len() as u16),
        ..rows[3]
    };
    for (index, branch) in UPDATE_CHANNELS
        .iter()
        .enumerate()
        .take(list.height as usize)
    {
        let row = Rect {
            x: list.x,
            y: list.y + index as u16,
            width: list.width,
            height: 1,
        };
        let selected = index == app.channel_cursor;
        let current = *branch == app.update_channel();
        if selected {
            frame.render_widget(
                Block::default().style(Style::default().bg(theme.surface_alt)),
                row,
            );
        }

        let mut name_style = Style::default().fg(theme.text);
        if selected || current {
            name_style = name_style.add_modifier(Modifier::BOLD);
        }
        let description = match *branch {
            "main" => "stable releases",
            "development" => "newer builds, less stable",
            _ => "alternate channel",
        };
        let mut spans = vec![
            Span::styled(
                if selected { " › " } else { "   " },
                Style::default()
                    .fg(if selected { theme.accent } else { theme.border })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if current { "● " } else { "○ " },
                Style::default().fg(if current { theme.positive } else { theme.muted }),
            ),
            Span::styled(format!("{branch:<13}"), name_style),
            Span::styled(description, Style::default().fg(theme.muted)),
        ];
        if current {
            spans.push(Span::styled(
                "  · current",
                Style::default()
                    .fg(theme.positive)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), row);
    }

    frame.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(content.width as usize),
            Style::default().fg(theme.border),
        )),
        rows[5],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("CURRENT  ", Style::default().fg(theme.muted)),
            Span::styled(
                app.update_channel().to_string(),
                Style::default()
                    .fg(theme.positive)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  ·  remembered by the host after install",
                Style::default().fg(theme.muted),
            ),
        ])),
        rows[6],
    );

    let highlighted = app.highlighted_channel();
    let action = if highlighted == app.update_channel() {
        Line::from(vec![
            Span::styled(
                " ENTER",
                Style::default()
                    .fg(theme.positive)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" close · {highlighted} is already the current channel"),
                Style::default().fg(theme.text),
            ),
        ])
    } else if app.update_enabled() {
        Line::from(vec![
            Span::styled(" ENTER", action_badge(theme)),
            Span::styled(
                format!(" install latest {highlighted} build"),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
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
                "no update installer is attached to this session",
                Style::default().fg(theme.muted),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(action).wrap(Wrap { trim: true }), rows[7]);

    let hint = if highlighted == app.update_channel() {
        "↑/↓ choose · enter close · esc close"
    } else {
        "↑/↓ choose · enter install · esc close"
    };
    frame.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(theme.muted))),
        rows[8],
    );
}

pub fn render_prompt(frame: &mut Frame, screen: Rect, app: &mut App) {
    let Some(notice) = app.update_notice() else {
        return;
    };
    let theme = app.theme;
    let stage = stage(app, screen);
    let area = widgets::centered_rect_height(stage, 78, 11);
    frame.render_widget(Clear, area);
    let title = format!("Update available · {} channel", notice.info().branch());
    let block = widgets::accent_panel(&title, &theme, theme.warning);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let versions = Line::from(vec![
        Span::styled("CURRENT  ", Style::default().fg(theme.muted)),
        Span::styled(
            notice.info().current().to_string(),
            Style::default().fg(theme.text),
        ),
        Span::styled("  →  ", Style::default().fg(theme.border)),
        Span::styled("LATEST  ", Style::default().fg(theme.muted)),
        Span::styled(
            notice.info().latest().to_string(),
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let source = Line::from(Span::styled(
        format!(
            "A newer build is published on the {} branch.",
            notice.info().branch()
        ),
        Style::default().fg(theme.muted),
    ));
    frame.render_widget(
        Paragraph::new(Text::from(vec![versions, source])),
        Rect {
            height: 2,
            ..rows[0]
        },
    );

    let notes = notice
        .info()
        .notes()
        .map(str::to_string)
        .unwrap_or_else(|| "No release notes were provided for this build.".to_string());
    frame.render_widget(
        Paragraph::new(notes)
            .style(Style::default().fg(theme.text))
            .wrap(Wrap { trim: true }),
        rows[1],
    );

    let url = notice
        .info()
        .url()
        .map(str::to_string)
        .unwrap_or_else(|| format!("Source: GitHub {}", notice.info().branch()));
    frame.render_widget(
        Paragraph::new(Span::styled(
            widgets::truncate(&url, inner.width),
            Style::default().fg(theme.muted),
        )),
        rows[2],
    );

    frame.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(inner.width as usize),
            Style::default().fg(theme.border),
        )),
        rows[3],
    );

    let enabled = app.update_enabled();
    let install_label = " Install update";
    let action = if enabled {
        Line::from(vec![
            Span::styled(" ENTER", action_badge(&theme)),
            Span::styled(
                install_label,
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                "INSTALL UNAVAILABLE ",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "No update handler is attached to this session.",
                Style::default().fg(theme.muted),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(action).wrap(Wrap { trim: true }), rows[4]);

    // The visible Install control is one click target. It exists only while
    // the prompt is open and an installer is attached.
    if enabled && rows[4].height > 0 && inner.width > 0 {
        let width = (" ENTER".chars().count() + install_label.chars().count()) as u16;
        app.hits.update_install = Some(Rect {
            x: rows[4].x,
            y: rows[4].y,
            width: width.min(rows[4].width),
            height: 1,
        });
    }

    let hint = if enabled {
        "Click Install or press Enter · terminal suspends · Esc later"
    } else {
        "Esc close · relaunch AMPH with an update handler to install"
    };
    frame.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(theme.muted))),
        rows[5],
    );
}

pub fn render_busy(frame: &mut Frame, screen: Rect, app: &App) {
    let theme = &app.theme;
    let stage = stage(app, screen);
    let area = widgets::centered_rect_height(stage, 60, 7);
    frame.render_widget(Clear, area);
    let block = widgets::accent_panel("Update installer", theme, theme.info);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let lines = vec![
        Line::from(Span::styled(
            "Terminal suspended · installer is running",
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Installer output returns here when it finishes.",
            Style::default().fg(theme.muted),
        )),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: true }),
        inner,
    );
}

pub fn render_result(frame: &mut Frame, screen: Rect, app: &App, state: &UpdateResultState) {
    let theme = &app.theme;
    let stage = stage(app, screen);
    let height = stage.height.saturating_sub(2).clamp(11, 24);
    let area = widgets::centered_rect_height(stage, 88, height);
    frame.render_widget(Clear, area);
    let (title, accent) = if state.success {
        ("Update installed · restart AMPH", theme.positive)
    } else {
        ("Update not installed", theme.danger)
    };
    let block = widgets::accent_panel(title, theme, accent);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(2),
        Constraint::Length(1),
    ])
    .split(inner);

    let mut header = vec![Span::styled(
        if state.success {
            format!("✓ {} installed", state.version)
        } else {
            format!("✕ {} was not installed", state.version)
        },
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )];
    if let Some(note) = &state.note {
        header.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
        header.push(Span::styled(
            widgets::truncate(note, inner.width.saturating_sub(24)),
            Style::default().fg(theme.muted),
        ));
    }
    let status = if state.success {
        "RESTART AMPH to load the new version · quit this session, then relaunch.".to_string()
    } else {
        "The running version is unchanged. Review the installer output below.".to_string()
    };
    let status_color = if state.success {
        theme.positive
    } else {
        theme.warning
    };
    let header_paragraph = Paragraph::new(Text::from(vec![
        Line::from(header),
        Line::from(Span::styled(
            status,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        )),
    ]))
    .wrap(Wrap { trim: true });
    frame.render_widget(
        header_paragraph,
        Rect {
            height: 2,
            ..rows[0]
        },
    );

    super::overlays::render_output_block(
        frame,
        rows[1],
        &state.output,
        state.scroll,
        theme,
        "Installer output",
    );

    let hint = if state.success {
        format!(
            "↑/↓ scroll · Enter close · restart AMPH to load {}",
            state.version
        )
    } else {
        "↑/↓ scroll · Esc close · u retry".to_string()
    };
    frame.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(theme.muted))),
        rows[2],
    );
}

fn stage(app: &App, screen: Rect) -> Rect {
    if app.layout.body.height >= 16 {
        app.layout.body
    } else {
        screen
    }
}

fn action_badge(theme: &Theme) -> Style {
    if theme.is_monochrome() {
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.bg)
            .bg(theme.positive)
            .add_modifier(Modifier::BOLD)
    }
}
